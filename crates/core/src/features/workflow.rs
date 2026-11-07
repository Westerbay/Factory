use super::*;
use std::{collections::BTreeSet, path::Path};

impl NewFeature {
    pub fn validate(&self) -> Result<(), FeatureError> {
        require(
            !self.operation_id.is_nil()
                && !self.project_id.is_nil()
                && text(&self.provider, 64)
                && self.provider.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'_')
                })
                && text(&self.workflow_revision, 128)
                && text(&self.title, 256)
                && text(&self.opener, 8_192),
            FeatureError::Invalid,
        )
    }
}

impl FeatureState {
    pub fn new(feature: Feature) -> Self {
        Self {
            feature,
            revisions: Vec::new(),
            scope: None,
            architecture: None,
            attempts: Vec::new(),
            reviews: Vec::new(),
            current_review: None,
            additional_review_rounds: 0,
            deliveries: Vec::new(),
        }
    }

    /// Run under the feature row lock after the store checks authorization, the
    /// current environment and operation replay. A rejected command changes nothing.
    pub fn apply(
        &mut self,
        actor_id: Uuid,
        generation: i64,
        operation_id: Uuid,
        command: FeatureCommand,
        now: DateTime<Utc>,
    ) -> Result<(), FeatureError> {
        require(self.feature.user_id == actor_id, FeatureError::Unauthorized)?;
        require(
            generation > 0 && !operation_id.is_nil(),
            FeatureError::Invalid,
        )?;
        match command {
            FeatureCommand::ProposeRevision(revision) => self.propose(revision),
            FeatureCommand::ApproveRevision(key) => self.approve(
                key,
                GateApproval {
                    user_id: actor_id,
                    at: now,
                },
            ),
            FeatureCommand::ClaimAttempt(claim) => self.claim(claim, generation),
            FeatureCommand::RecordAttempt { attempt_id, update } => {
                self.record_attempt(attempt_id, generation, update)
            }
            FeatureCommand::RecoverAttempt {
                attempt_id,
                launch_generation,
            } => self.recover_attempt(attempt_id, launch_generation, generation, None),
            FeatureCommand::RecoverObservedAttempt {
                attempt_id,
                launch_generation,
                worker_id,
            } => self.recover_attempt(attempt_id, launch_generation, generation, Some(worker_id)),
            FeatureCommand::SaveRetryBase {
                attempt_id,
                base_oid,
            } => self.save_retry_base(attempt_id, generation, base_oid),
            FeatureCommand::BeginReview {
                snapshot,
                required_roles,
            } => self.begin_review(snapshot, required_roles),
            FeatureCommand::ClaimIntegratedCheck {
                snapshot_id,
                command,
                workspace_id,
            } => self.claim_integrated_check(snapshot_id, command, workspace_id, generation),
            FeatureCommand::StartIntegratedCheck {
                snapshot_id,
                workspace_id,
            } => {
                let check = self.integrated_check_mut(snapshot_id, workspace_id)?;
                require(
                    check.status == IntegratedCheckStatus::Preparing,
                    FeatureError::Conflict,
                )?;
                check.generation = generation;
                check.status = IntegratedCheckStatus::Launching;
                Ok(())
            }
            FeatureCommand::FinishIntegratedCheck {
                snapshot_id,
                workspace_id,
                passed,
            } => {
                let check = self.integrated_check_mut(snapshot_id, workspace_id)?;
                require(
                    check.status == IntegratedCheckStatus::Launching
                        && generation >= check.generation,
                    FeatureError::Conflict,
                )?;
                check.status = IntegratedCheckStatus::Finished { passed };
                Ok(())
            }
            FeatureCommand::RecordReview {
                snapshot_id,
                patch_digest,
                verdict,
            } => self.record_review(snapshot_id, &patch_digest, verdict),
            FeatureCommand::BeginCorrections { snapshot_id } => {
                self.ensure_idle()?;
                let review = self.active_review(snapshot_id)?;
                require(
                    self.feature.stage == FeatureStage::Review
                        && review
                            .verdict
                            .as_ref()
                            .is_some_and(|verdict| !verdict.ready()),
                    FeatureError::Conflict,
                )?;
                self.feature.stage = FeatureStage::Execution;
                Ok(())
            }
            FeatureCommand::AuthorizeAdditionalReview { snapshot_id } => {
                self.active_review(snapshot_id)?;
                require(
                    matches!(
                        self.feature.stage,
                        FeatureStage::Review | FeatureStage::Execution
                    ) && !self.review_slot_available(),
                    FeatureError::Conflict,
                )?;
                self.additional_review_rounds = self
                    .additional_review_rounds
                    .checked_add(1)
                    .ok_or(FeatureError::ReviewBlocked)?;
                Ok(())
            }
            FeatureCommand::ClaimDelivery {
                snapshot_id,
                observed,
            } => self.claim_delivery(operation_id, snapshot_id, &observed),
            FeatureCommand::RecordDelivery { request_id, status } => {
                self.record_delivery(request_id, status)
            }
            FeatureCommand::InvalidateResult { snapshot_id } => {
                let ready = self
                    .active_review(snapshot_id)?
                    .verdict
                    .as_ref()
                    .is_some_and(ReviewVerdict::ready);
                require(
                    self.feature.stage == FeatureStage::Result
                        || self.feature.stage == FeatureStage::Review && ready,
                    FeatureError::Conflict,
                )?;
                self.ensure_idle()?;
                self.feature.stage = FeatureStage::Review;
                Ok(())
            }
        }
    }

    pub fn current_scope(&self) -> Option<&GateRevision> {
        self.scope.as_ref().and_then(|key| self.revision(key))
    }

    pub fn current_architecture(&self) -> Option<&GateRevision> {
        self.architecture
            .as_ref()
            .and_then(|key| self.revision(key))
    }

    pub fn review(&self) -> Option<&ReviewRound> {
        self.current_review
            .and_then(|id| self.reviews.iter().find(|round| round.snapshot.id == id))
    }

    fn revision(&self, key: &RevisionKey) -> Option<&GateRevision> {
        self.revisions.iter().find(|revision| revision.key == *key)
    }

    /// Validate the member-facing proposal without requiring an idle run. The
    /// eventual transition still checks live execution and delivery reservations.
    pub fn validate_proposal(&self, revision: &GateRevision) -> Result<(), FeatureError> {
        require(
            self.feature.stage != FeatureStage::Draft,
            FeatureError::Conflict,
        )?;
        require(
            revision.key.valid() && revision.approval.is_none(),
            FeatureError::Invalid,
        )?;
        require(
            !self
                .revisions
                .iter()
                .any(|existing| existing.key.id == revision.key.id),
            FeatureError::Conflict,
        )?;
        match &revision.content {
            GateContent::Scope { brief } => require(text(brief, 65_536), FeatureError::Invalid),
            GateContent::Architecture {
                scope,
                summary,
                diagrams,
                tasks,
            } => {
                require(
                    self.current_scope().is_some_and(|revision| {
                        revision.key == *scope && revision.approval.is_some()
                    }),
                    FeatureError::Conflict,
                )?;
                require(
                    text(summary, 32_768)
                        && !diagrams.is_empty()
                        && diagrams.len() <= 12
                        && diagrams.iter().all(|diagram| {
                            text(&diagram.title, 256) && text(&diagram.source, 65_536)
                        }),
                    FeatureError::Invalid,
                )?;
                validate_task_graph(tasks)
            }
        }
    }

    fn propose(&mut self, revision: GateRevision) -> Result<(), FeatureError> {
        self.ensure_idle()?;
        self.validate_proposal(&revision)?;
        match revision.content {
            GateContent::Scope { .. } => {
                self.scope = Some(revision.key.clone());
                self.architecture = None;
                self.feature.stage = FeatureStage::Scope;
            }
            GateContent::Architecture { .. } => {
                self.architecture = Some(revision.key.clone());
                self.feature.stage = FeatureStage::Architecture;
            }
        }
        self.current_review = None;
        self.additional_review_rounds = 0;
        self.revisions.push(revision);
        Ok(())
    }

    fn approve(&mut self, key: RevisionKey, approval: GateApproval) -> Result<(), FeatureError> {
        self.ensure_idle()?;
        let next = match self.feature.stage {
            FeatureStage::Scope if self.scope.as_ref() == Some(&key) => FeatureStage::Architecture,
            FeatureStage::Architecture if self.architecture.as_ref() == Some(&key) => {
                FeatureStage::Execution
            }
            _ => return Err(FeatureError::Conflict),
        };
        let revision = self
            .revisions
            .iter_mut()
            .find(|revision| revision.key == key && revision.approval.is_none())
            .ok_or(FeatureError::Conflict)?;
        revision.approval = Some(approval);
        self.feature.stage = next;
        Ok(())
    }

    fn claim(&mut self, claim: AttemptClaim, generation: i64) -> Result<(), FeatureError> {
        require(
            claim.workspace.feature_id == self.feature.id
                && !claim.workspace.attempt_id.is_nil()
                && valid_oid(&claim.workspace.base_oid)
                && claim.workspace.access == claim.task.role.access()
                && text(&claim.model, 256)
                && text(&claim.skill_revision, 256)
                && claim
                    .effort
                    .as_deref()
                    .is_none_or(crate::conversations::execution::valid_effort)
                && claim.task.valid(),
            FeatureError::Invalid,
        )?;
        require(
            !self
                .attempts
                .iter()
                .any(|attempt| attempt.id() == claim.workspace.attempt_id),
            FeatureError::Conflict,
        )?;
        self.validate_assignment(&claim)?;
        if let Some(previous) = self.latest_attempt(&claim) {
            require(
                matches!(
                    previous.status,
                    AttemptStatus::Failed | AttemptStatus::Interrupted
                ) && previous.launch_settled
                    && claim.retry_of == Some(previous.id())
                    && claim.task == previous.claim.task
                    && claim.workspace.base_oid
                        == previous
                            .checkpoint
                            .as_ref()
                            .unwrap_or(&previous.claim.workspace.base_oid)
                            .as_str(),
                if previous.status.holds_reservation() {
                    FeatureError::ExecutionUncertain
                } else {
                    FeatureError::Conflict
                },
            )?;
        } else {
            require(claim.retry_of.is_none(), FeatureError::Conflict)?;
        }
        require(
            self.attempts.iter().all(|existing| {
                !existing.status.holds_reservation()
                    || match (existing.claim.workspace.access, claim.workspace.access) {
                        (WorkspaceAccess::Write, WorkspaceAccess::Write) => {
                            !existing.claim.task.paths.iter().any(|old| {
                                claim.task.paths.iter().any(|new| scopes_overlap(old, new))
                            })
                        }
                        _ => true,
                    }
            }),
            FeatureError::Conflict,
        )?;
        self.attempts.push(TaskAttempt {
            claim,
            generation,
            status: AttemptStatus::Starting,
            launch_settled: false,
            process_receipt: None,
            provider_thread: None,
            observed_model: None,
            activity: None,
            result: None,
            checkpoint: None,
            pending_verification: None,
        });
        Ok(())
    }

    fn validate_assignment(&self, claim: &AttemptClaim) -> Result<(), FeatureError> {
        let task = &claim.task;
        match self.feature.stage {
            FeatureStage::Scope => require(
                task.role == TaskRole::Coordinator
                    && claim.revision == self.scope
                    && claim.snapshot_id.is_none(),
                FeatureError::Conflict,
            ),
            FeatureStage::Architecture => {
                let expected = match task.role {
                    TaskRole::Coordinator | TaskRole::Architecture => self.scope.as_ref(),
                    TaskRole::ArchitectureReview => self.architecture.as_ref(),
                    _ => None,
                };
                require(
                    expected.is_some()
                        && claim.revision.as_ref() == expected
                        && claim.snapshot_id.is_none(),
                    FeatureError::Conflict,
                )
            }
            FeatureStage::Execution => self.validate_execution_task(claim),
            FeatureStage::Review => {
                let review = claim
                    .snapshot_id
                    .ok_or(FeatureError::Conflict)
                    .and_then(|id| self.active_review(id))?;
                require(
                    claim.revision.as_ref() == Some(&review.architecture)
                        && review.verdict.is_none()
                        && review.required_roles.contains(&task.role)
                        && claim.workspace.base_oid == review.snapshot.git.head_oid
                        && self.attempts.iter().all(|attempt| {
                            attempt.claim.snapshot_id != claim.snapshot_id
                                || attempt.claim.task.role != task.role
                                || attempt.claim.task.id == task.id
                        }),
                    FeatureError::Conflict,
                )
            }
            _ => Err(FeatureError::Conflict),
        }
    }

    fn validate_execution_task(&self, claim: &AttemptClaim) -> Result<(), FeatureError> {
        let (architecture, tasks) = self.approved_tasks()?;
        let task = &claim.task;
        require(
            claim.revision.as_ref() == Some(architecture),
            FeatureError::Conflict,
        )?;
        if task.role == TaskRole::Correction {
            let correctable = match claim.snapshot_id {
                Some(id) => self
                    .active_review(id)?
                    .verdict
                    .as_ref()
                    .is_some_and(|verdict| !verdict.ready()),
                None => self.current_review.is_none(),
            };
            require(
                correctable
                    && task.depends_on.is_empty()
                    && task.paths.iter().all(|path| {
                        tasks
                            .iter()
                            .flat_map(|task| &task.paths)
                            .any(|scope| covers(scope, path))
                    }),
                FeatureError::Conflict,
            )
        } else {
            require(
                claim.snapshot_id.is_none()
                    && tasks.contains(task)
                    && task
                        .depends_on
                        .iter()
                        .all(|id| self.task_validated(architecture, *id)),
                FeatureError::Conflict,
            )
        }
    }

    fn approved_tasks(&self) -> Result<(&RevisionKey, &[FeatureTask]), FeatureError> {
        let revision = self
            .current_architecture()
            .filter(|revision| revision.approval.is_some())
            .ok_or(FeatureError::Conflict)?;
        match &revision.content {
            GateContent::Architecture { tasks, .. } => Ok((&revision.key, tasks)),
            _ => Err(FeatureError::Conflict),
        }
    }

    fn task_validated(&self, architecture: &RevisionKey, task_id: Uuid) -> bool {
        self.attempts
            .iter()
            .rev()
            .find(|attempt| {
                attempt.claim.revision.as_ref() == Some(architecture)
                    && attempt.claim.task.id == task_id
                    && attempt.claim.snapshot_id.is_none()
            })
            .is_some_and(|attempt| attempt.status == AttemptStatus::Validated)
    }

    fn latest_attempt(&self, claim: &AttemptClaim) -> Option<&TaskAttempt> {
        self.attempts.iter().rev().find(|attempt| {
            attempt.claim.task.id == claim.task.id
                && attempt.claim.revision == claim.revision
                && attempt.claim.snapshot_id == claim.snapshot_id
        })
    }

    fn record_attempt(
        &mut self,
        id: Uuid,
        generation: i64,
        update: AttemptUpdate,
    ) -> Result<(), FeatureError> {
        let index = self
            .attempts
            .iter()
            .position(|attempt| attempt.id() == id && attempt.generation == generation)
            .ok_or(FeatureError::Conflict)?;
        let (earlier, current) = self.attempts.split_at_mut(index);
        current[0].record(update, earlier)
    }

    fn recover_attempt(
        &mut self,
        id: Uuid,
        launch_generation: i64,
        generation: i64,
        worker_id: Option<String>,
    ) -> Result<(), FeatureError> {
        if let Some(id) = &worker_id {
            require(hex(id, 64), FeatureError::Invalid)?;
        }
        let attempt = self
            .attempts
            .iter_mut()
            .find(|attempt| attempt.id() == id && attempt.generation == launch_generation)
            .ok_or(FeatureError::Conflict)?;
        require(
            generation >= launch_generation && attempt.status.holds_reservation(),
            FeatureError::Conflict,
        )?;
        if let Some(worker_id) = worker_id {
            if !attempt.launch_settled {
                attempt.launch_settled = true;
                attempt.process_receipt = Some(worker_id);
            }
            attempt.pending_verification = None;
        } else {
            require(
                attempt.launch_settled && attempt.pending_verification.is_none(),
                FeatureError::ExecutionUncertain,
            )?;
        }
        attempt.status = AttemptStatus::Interrupted;
        Ok(())
    }

    fn save_retry_base(
        &mut self,
        id: Uuid,
        generation: i64,
        base_oid: String,
    ) -> Result<(), FeatureError> {
        require(valid_oid(&base_oid), FeatureError::Invalid)?;
        let index = self
            .attempts
            .iter()
            .position(|attempt| attempt.id() == id)
            .ok_or(FeatureError::Conflict)?;
        let attempt = &self.attempts[index];
        require(
            generation >= attempt.generation
                && attempt.launch_settled
                && matches!(
                    attempt.status,
                    AttemptStatus::Failed | AttemptStatus::Interrupted
                )
                && attempt.claim.workspace.access == WorkspaceAccess::Write
                && attempt.checkpoint.is_none()
                && base_oid != attempt.claim.workspace.base_oid
                && self
                    .latest_attempt(&attempt.claim)
                    .is_some_and(|latest| latest.id() == id),
            FeatureError::Conflict,
        )?;
        self.validate_assignment(&attempt.claim)?;
        self.attempts[index].checkpoint = Some(base_oid);
        Ok(())
    }

    fn begin_review(
        &mut self,
        snapshot: ReviewSnapshot,
        required_roles: Vec<TaskRole>,
    ) -> Result<(), FeatureError> {
        self.ensure_idle()?;
        require(
            matches!(
                self.feature.stage,
                FeatureStage::Execution | FeatureStage::Review
            ) && self.review().is_none_or(|review| review.verdict.is_some()),
            FeatureError::Conflict,
        )?;
        require(
            !snapshot.id.is_nil()
                && snapshot.git.valid()
                && required_roles.contains(&TaskRole::GeneralReview)
                && required_roles.iter().all(|role| role.is_reviewer())
                && required_roles.iter().collect::<BTreeSet<_>>().len() == required_roles.len(),
            FeatureError::Invalid,
        )?;
        require(
            self.reviews
                .iter()
                .all(|review| review.snapshot.id != snapshot.id),
            FeatureError::Conflict,
        )?;
        if let Some(round) = self.review()
            && let Some(verdict) = round.verdict.as_ref().filter(|verdict| !verdict.ready())
        {
            let correction = self.attempts.iter().rev().find(|attempt| {
                attempt.claim.snapshot_id == Some(round.snapshot.id)
                    && attempt.claim.task.role == TaskRole::Correction
            });
            require(
                correction.is_some_and(|attempt| attempt.status == AttemptStatus::Validated)
                    && (!verdict.findings.iter().any(|finding| finding.blocking)
                        || round.snapshot.git.patch_digest != snapshot.git.patch_digest),
                FeatureError::ReviewBlocked,
            )?;
        }
        let (architecture, tasks) = self.approved_tasks()?;
        require(
            self.review_slot_available()
                && tasks
                    .iter()
                    .all(|task| self.task_validated(architecture, task.id))
                && self.attempts.iter().all(|attempt| {
                    attempt.claim.revision.as_ref() != Some(architecture)
                        || attempt.claim.task.role != TaskRole::Correction
                        || self
                            .latest_attempt(&attempt.claim)
                            .is_some_and(|latest| latest.status == AttemptStatus::Validated)
                }),
            FeatureError::ReviewBlocked,
        )?;
        let architecture = architecture.clone();
        self.current_review = Some(snapshot.id);
        self.reviews.push(ReviewRound {
            snapshot,
            architecture,
            required_roles,
            verdict: None,
            integrated_checks: Vec::new(),
        });
        self.feature.stage = FeatureStage::Review;
        Ok(())
    }

    fn claim_integrated_check(
        &mut self,
        snapshot_id: Uuid,
        command: String,
        workspace_id: Uuid,
        generation: i64,
    ) -> Result<(), FeatureError> {
        require(
            self.feature.stage == FeatureStage::Review && !workspace_id.is_nil(),
            FeatureError::Conflict,
        )?;
        let round = self.active_review(snapshot_id)?;
        let (_, tasks) = self.approved_tasks()?;
        require(
            tasks.iter().any(|task| task.checks.contains(&command)),
            FeatureError::Invalid,
        )?;
        require(
            round.verdict.is_none()
                && round.integrated_checks.iter().all(|check| {
                    !check.status.holds_reservation()
                        && check.command != command
                        && check.workspace_id != workspace_id
                }),
            FeatureError::Conflict,
        )?;
        self.reviews
            .iter_mut()
            .find(|review| review.snapshot.id == snapshot_id)
            .ok_or(FeatureError::Conflict)?
            .integrated_checks
            .push(IntegratedCheck {
                command,
                workspace_id,
                generation,
                status: IntegratedCheckStatus::Preparing,
            });
        Ok(())
    }

    fn integrated_check_mut(
        &mut self,
        snapshot_id: Uuid,
        workspace_id: Uuid,
    ) -> Result<&mut IntegratedCheck, FeatureError> {
        require(
            self.feature.stage == FeatureStage::Review,
            FeatureError::Conflict,
        )?;
        require(
            self.active_review(snapshot_id)?.verdict.is_none(),
            FeatureError::Conflict,
        )?;
        self.reviews
            .iter_mut()
            .find(|review| review.snapshot.id == snapshot_id)
            .and_then(|review| {
                review
                    .integrated_checks
                    .iter_mut()
                    .find(|check| check.workspace_id == workspace_id)
            })
            .ok_or(FeatureError::Conflict)
    }

    fn record_review(
        &mut self,
        snapshot_id: Uuid,
        patch_digest: &str,
        verdict: ReviewVerdict,
    ) -> Result<(), FeatureError> {
        require(
            self.feature.stage == FeatureStage::Review,
            FeatureError::Conflict,
        )?;
        let round = self.active_review(snapshot_id)?;
        require(
            round.snapshot.git.patch_digest == patch_digest,
            FeatureError::StaleSnapshot,
        )?;
        require(round.verdict.is_none(), FeatureError::Conflict)?;
        require(
            round.required_roles.iter().all(|role| {
                self.attempts.iter().any(|attempt| {
                    attempt.claim.snapshot_id == Some(snapshot_id)
                        && attempt.claim.task.role == *role
                        && attempt.status == AttemptStatus::Validated
                })
            }),
            FeatureError::ReviewBlocked,
        )?;
        let (_, tasks) = self.approved_tasks()?;
        let approved_checks = tasks
            .iter()
            .flat_map(|task| &task.checks)
            .collect::<BTreeSet<_>>();
        let reported_checks = verdict
            .checks
            .iter()
            .map(|check| &check.label)
            .collect::<BTreeSet<_>>();
        require(
            reported_checks.len() == verdict.checks.len()
                && reported_checks.is_subset(&approved_checks),
            FeatureError::Invalid,
        )?;
        require(
            approved_checks == reported_checks
                && verdict.checks.iter().all(|reported| {
                    round.integrated_checks.iter().any(|check| {
                        check.command == reported.label
                            && check.status
                                == IntegratedCheckStatus::Finished {
                                    passed: reported.passed,
                                }
                    })
                }),
            FeatureError::ReviewBlocked,
        )?;
        require(
            verdict.findings.len() <= 128
                && verdict.findings.iter().all(|finding| {
                    round.required_roles.contains(&finding.role) && text(&finding.summary, 16_384)
                })
                && verdict.checks.len() <= 128
                && verdict.checks.iter().all(|check| text(&check.label, 4096)),
            FeatureError::Invalid,
        )?;
        if verdict.ready() {
            self.feature.stage = FeatureStage::Result;
        }
        self.reviews
            .iter_mut()
            .find(|review| review.snapshot.id == snapshot_id)
            .ok_or(FeatureError::Conflict)?
            .verdict = Some(verdict);
        Ok(())
    }

    fn active_review(&self, id: Uuid) -> Result<&ReviewRound, FeatureError> {
        self.review()
            .filter(|review| review.snapshot.id == id)
            .ok_or(FeatureError::StaleSnapshot)
    }

    fn review_slot_available(&self) -> bool {
        let used = self
            .reviews
            .iter()
            .filter(|review| self.architecture.as_ref() == Some(&review.architecture))
            .count();
        used < 2 + self.additional_review_rounds as usize
    }

    fn claim_delivery(
        &mut self,
        request_id: Uuid,
        snapshot_id: Uuid,
        observed: &GitSnapshot,
    ) -> Result<(), FeatureError> {
        self.ensure_idle()?;
        let review = self.active_review(snapshot_id)?;
        require(
            review.snapshot.git == *observed,
            FeatureError::StaleSnapshot,
        )?;
        require(
            self.feature.stage == FeatureStage::Result
                && review.verdict.as_ref().is_some_and(ReviewVerdict::ready)
                && self.deliveries.iter().all(|receipt| {
                    receipt.request_id != request_id
                        && !matches!(
                            receipt.status,
                            DeliveryStatus::Published { .. }
                                | DeliveryStatus::PublishedStale { .. }
                        )
                }),
            FeatureError::Conflict,
        )?;
        self.deliveries.push(DeliveryReceipt {
            request_id,
            snapshot_id,
            status: DeliveryStatus::Claimed,
        });
        Ok(())
    }

    fn record_delivery(
        &mut self,
        request_id: Uuid,
        status: DeliveryStatus,
    ) -> Result<(), FeatureError> {
        let receipt = self
            .deliveries
            .iter_mut()
            .find(|receipt| {
                receipt.request_id == request_id
                    && matches!(
                        receipt.status,
                        DeliveryStatus::Claimed | DeliveryStatus::Uncertain
                    )
            })
            .ok_or(FeatureError::Conflict)?;
        match &status {
            DeliveryStatus::Claimed => return Err(FeatureError::Invalid),
            DeliveryStatus::Uncertain => {}
            DeliveryStatus::Failed { reason } => {
                require(text(reason, 4096), FeatureError::Invalid)?
            }
            DeliveryStatus::Published { draft_url }
            | DeliveryStatus::PublishedStale { draft_url } => {
                require(text(draft_url, 2048), FeatureError::Invalid)?;
                self.feature.stage = if matches!(status, DeliveryStatus::Published { .. }) {
                    FeatureStage::Draft
                } else {
                    FeatureStage::Review
                };
            }
        }
        receipt.status = status;
        Ok(())
    }

    fn ensure_idle(&self) -> Result<(), FeatureError> {
        require(
            !self
                .attempts
                .iter()
                .any(|attempt| attempt.status.holds_reservation())
                && !self.reviews.iter().any(|round| {
                    round
                        .integrated_checks
                        .iter()
                        .any(|check| check.status.holds_reservation())
                })
                && !self.deliveries.iter().any(|receipt| {
                    matches!(
                        receipt.status,
                        DeliveryStatus::Claimed | DeliveryStatus::Uncertain
                    )
                }),
            FeatureError::ExecutionUncertain,
        )
    }
}

impl TaskAttempt {
    fn record(
        &mut self,
        update: AttemptUpdate,
        earlier: &[TaskAttempt],
    ) -> Result<(), FeatureError> {
        require(self.status.holds_reservation(), FeatureError::Conflict)?;
        match update {
            AttemptUpdate::LaunchSettled { process_receipt } => {
                require(
                    !self.launch_settled
                        && process_receipt
                            .as_deref()
                            .is_none_or(|receipt| text(receipt, 512)),
                    FeatureError::Conflict,
                )?;
                self.launch_settled = true;
                self.process_receipt = process_receipt;
                if self.status == AttemptStatus::Starting && self.process_receipt.is_some() {
                    self.status = AttemptStatus::Running;
                }
            }
            AttemptUpdate::ProviderThread { id } => {
                require(
                    self.status == AttemptStatus::Running,
                    FeatureError::Conflict,
                )?;
                require(text(&id, 512), FeatureError::Invalid)?;
                require(
                    self.provider_thread
                        .as_ref()
                        .is_none_or(|prior| prior == &id),
                    FeatureError::Conflict,
                )?;
                self.provider_thread = Some(id);
            }
            AttemptUpdate::ObservedModel { model } => {
                require(
                    self.status == AttemptStatus::Running,
                    FeatureError::Conflict,
                )?;
                require(text(&model, 256), FeatureError::Invalid)?;
                self.observed_model = Some(model);
            }
            AttemptUpdate::Activity { summary } => {
                require(
                    self.status == AttemptStatus::Running,
                    FeatureError::Conflict,
                )?;
                require(text(&summary, 4096), FeatureError::Invalid)?;
                self.activity = Some(summary);
            }
            AttemptUpdate::Result { text: result } => {
                require(
                    self.status == AttemptStatus::Running && self.result.is_none(),
                    FeatureError::Conflict,
                )?;
                require(text(&result, 32_768), FeatureError::Invalid)?;
                self.result = Some(result);
            }
            AttemptUpdate::VerificationStarted { workspace_id } => {
                require(
                    self.status == AttemptStatus::Running
                        && self.launch_settled
                        && self.pending_verification.is_none()
                        && !workspace_id.is_nil()
                        && !self.claim.task.checks.is_empty(),
                    FeatureError::Conflict,
                )?;
                self.pending_verification = Some(workspace_id);
            }
            AttemptUpdate::VerificationSettled => {
                require(
                    self.status == AttemptStatus::Running && self.pending_verification.is_some(),
                    FeatureError::Conflict,
                )?;
                self.pending_verification = None;
            }
            AttemptUpdate::Validated { checkpoint } => {
                require(
                    self.status == AttemptStatus::Running
                        && self.pending_verification.is_none()
                        && (!self.claim.task.role.is_reviewer() || self.result.is_some()),
                    FeatureError::Conflict,
                )?;
                require(
                    match self.claim.workspace.access {
                        WorkspaceAccess::Write => checkpoint.as_deref().is_some_and(valid_oid),
                        WorkspaceAccess::ReadOnly => checkpoint.is_none(),
                    },
                    FeatureError::Invalid,
                )?;
                self.checkpoint = checkpoint;
                self.status = AttemptStatus::Validated;
            }
            AttemptUpdate::Failed { reason } => {
                require(
                    self.launch_settled
                        && self.pending_verification.is_none()
                        && self.status != AttemptStatus::Uncertain,
                    FeatureError::ExecutionUncertain,
                )?;
                require(text(&reason, 4096), FeatureError::Invalid)?;
                self.activity = Some(reason);
                self.status = AttemptStatus::Failed;
            }
            AttemptUpdate::IntegrationConflict { retry_base } => {
                require(
                    self.status == AttemptStatus::Running
                        && self.launch_settled
                        && self.pending_verification.is_none()
                        && self.claim.workspace.access == WorkspaceAccess::Write
                        && valid_oid(&retry_base)
                        && retry_base != self.claim.workspace.base_oid,
                    FeatureError::Conflict,
                )?;
                self.checkpoint = Some(retry_base);
                self.activity =
                    Some("Integration conflicted. Resume from the verified branch head.".into());
                self.status = AttemptStatus::Failed;
            }
            AttemptUpdate::DiffUnrenderable => {
                require(
                    self.status == AttemptStatus::Running
                        && self.launch_settled
                        && self.pending_verification.is_none()
                        && self.claim.workspace.access == WorkspaceAccess::Write,
                    FeatureError::Conflict,
                )?;
                // Earlier retries may start from retained scratch. Pin the next
                // attempt to the first verified base so that history stays out.
                let original = earlier
                    .iter()
                    .find(|attempt| {
                        attempt.claim.task.id == self.claim.task.id
                            && attempt.claim.revision == self.claim.revision
                            && attempt.claim.snapshot_id == self.claim.snapshot_id
                    })
                    .unwrap_or(self);
                self.checkpoint = Some(original.claim.workspace.base_oid.clone());
                self.activity = Some(WorkspaceError::DiffUnrenderable.to_string());
                self.status = AttemptStatus::Failed;
            }
            AttemptUpdate::Interrupted => {
                require(
                    self.launch_settled
                        && self.pending_verification.is_none()
                        && self.status != AttemptStatus::Uncertain,
                    FeatureError::ExecutionUncertain,
                )?;
                self.status = AttemptStatus::Interrupted;
            }
            AttemptUpdate::Uncertain => self.status = AttemptStatus::Uncertain,
        }
        Ok(())
    }
}

impl RevisionKey {
    fn valid(&self) -> bool {
        !self.id.is_nil() && hex(&self.digest, 64)
    }
}

impl GitSnapshot {
    fn valid(&self) -> bool {
        valid_oid(&self.base_oid) && valid_oid(&self.head_oid) && hex(&self.patch_digest, 64)
    }
}

impl FeatureTask {
    fn valid(&self) -> bool {
        !self.id.is_nil()
            && text(&self.title, 256)
            && text(&self.instructions, 32_768)
            && self.depends_on.len() <= 64
            && self.depends_on.iter().collect::<BTreeSet<_>>().len() == self.depends_on.len()
            && self.checks.len() <= 16
            && self.checks.iter().all(|command| text(command, 4096))
            && self.checks.iter().collect::<BTreeSet<_>>().len() == self.checks.len()
            && self.paths.len() <= 128
            && (self.role.access() == WorkspaceAccess::Write) != self.paths.is_empty()
            && self.paths.iter().all(|path| valid_path(path))
    }
}

fn validate_task_graph(tasks: &[FeatureTask]) -> Result<(), FeatureError> {
    let checks: BTreeSet<_> = tasks.iter().flat_map(|task| &task.checks).collect();
    require(
        !tasks.is_empty()
            && tasks.len() <= 64
            && (1..=128).contains(&checks.len())
            && tasks.iter().all(|task| {
                matches!(
                    task.role,
                    TaskRole::Implementation
                        | TaskRole::ComplexImplementation
                        | TaskRole::Visual
                        | TaskRole::Triage
                ) && task.valid()
            })
            && tasks
                .iter()
                .map(|task| task.id)
                .collect::<BTreeSet<_>>()
                .len()
                == tasks.len(),
        FeatureError::Invalid,
    )?;
    let mut visited = BTreeSet::new();
    loop {
        let prior = visited.len();
        for task in tasks {
            if task.depends_on.iter().all(|id| visited.contains(id)) {
                visited.insert(task.id);
            }
        }
        if visited.len() == tasks.len() {
            return Ok(());
        }
        if visited.len() == prior {
            return Err(FeatureError::Invalid);
        }
    }
}

fn covers(scope: &str, path: &str) -> bool {
    scope == path || scope.ends_with('/') && path.starts_with(scope)
}

fn scopes_overlap(left: &str, right: &str) -> bool {
    let left = left.trim_end_matches('/');
    let right = right.trim_end_matches('/');
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|rest| rest.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn valid_path(value: &str) -> bool {
    let path = value.strip_suffix('/').unwrap_or(value);
    text(path, 1024)
        && !path.contains(['\\', ':'])
        && !Path::new(path).is_absolute()
        && path
            .split('/')
            .all(|part| !matches!(part, "" | "." | "..") && !part.eq_ignore_ascii_case(".git"))
        && !path.chars().any(char::is_control)
}

fn valid_oid(value: &str) -> bool {
    hex(value, 40) || hex(value, 64)
}

fn hex(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.contains('\0')
}

fn require(condition: bool, error: FeatureError) -> Result<(), FeatureError> {
    if condition { Ok(()) } else { Err(error) }
}
