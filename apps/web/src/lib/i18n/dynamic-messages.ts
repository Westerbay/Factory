import * as m from "@workspace/i18n/messages"

// Controllers return these user-facing states as English message identifiers.
const messages = new Map<string, () => string>([
  [" (1M context)", m.m_1m_context],
  [
    "A file or folder with this repository name already exists in Documents. Rename or remove it before importing.",
    m.a_file_or_folder_with_this_repository_name_already_exists_in_docu,
  ],
  [
    "A previous task needs recovery before your workspace can start.",
    m.a_previous_task_needs_recovery_before_your_workspace_can_start,
  ],
  [
    "Access activated. The member must sign in again.",
    m.access_activated_the_member_must_sign_in_again,
  ],
  [
    "Access suspended and sessions revoked.",
    m.access_suspended_and_sessions_revoked,
  ],
  ["Active", m.active],
  [
    "Another sign-in may be active, or your environment changed. Check its status; no new sign-in was started automatically.",
    m.another_sign_in_may_be_active_or_your_environment_changed_check_i,
  ],
  [
    "Answer each question before submitting.",
    m.answer_each_question_before_submitting,
  ],
  [
    "Answer sent. Waiting for the provider to confirm receipt.",
    m.answer_sent_waiting_for_the_provider_to_confirm_receipt,
  ],
  [
    "Assistant could not complete this run. Review the conversation and try again.",
    m.assistant_could_not_complete_this_run_review_the_conversation_and,
  ],
  ["Assistant finished.", m.assistant_finished],
  ["Assistant is working…", m.assistant_is_working],
  ["Checking remaining imports…", m.checking_remaining_imports],
  ["Checking the import…", m.checking_the_import],
  [
    "Claude could not be disconnected. Check the connection and try again.",
    m.claude_could_not_be_disconnected_check_the_connection_and_try_aga,
  ],
  [
    "Claude is using an unsupported sign-in. Connect your personal Claude account.",
    m.claude_is_using_an_unsupported_sign_in_connect_your_personal_clau,
  ],
  [
    "Claude sign-in could not be updated. Check the connection and try again.",
    m.claude_sign_in_could_not_be_updated_check_the_connection_and_try,
  ],
  [
    "Claude status could not be checked. Try again.",
    m.claude_status_could_not_be_checked_try_again,
  ],
  ["Code copied.", m.code_copied],
  [
    "Codex did not confirm sign-in. Check the status before trying again.",
    m.codex_did_not_confirm_sign_in_check_the_status_before_trying_agai,
  ],
  [
    "Codex is unavailable. Check again in a moment.",
    m.codex_is_unavailable_check_again_in_a_moment,
  ],
  [
    "Deletion could not be confirmed. Check the project and retry if needed.",
    m.deletion_could_not_be_confirmed_check_the_project_and_retry_if_ne,
  ],
  [
    "Delivery could not be confirmed. Stop and recover the run if it remains blocked.",
    m.delivery_could_not_be_confirmed_stop_and_recover_the_run_if_it_re,
  ],
  [
    "Disconnected from the display. Your shell can keep running; reconnect to return.",
    m.disconnected_from_the_display_your_shell_can_keep_running_reconne,
  ],
  [
    "Environment setup was interrupted and needs administrator repair.",
    m.environment_setup_was_interrupted_and_needs_administrator_repair,
  ],
  ["Environment unavailable.", m.environment_unavailable],
  [
    "Environments need administrator configuration before they can be used.",
    m.environments_need_administrator_configuration_before_they_can_be,
  ],
  [
    "GitHub access could not be confirmed. Try connecting again.",
    m.github_access_could_not_be_confirmed_try_connecting_again,
  ],
  [
    "GitHub address not verified; check your Git remote before pushing.",
    m.github_address_not_verified_check_your_git_remote_before_pushing,
  ],
  [
    "GitHub authorization could not be confirmed.",
    m.github_authorization_could_not_be_confirmed,
  ],
  [
    "GitHub authorization could not be confirmed. Try connecting again.",
    m.github_authorization_could_not_be_confirmed_try_connecting_again,
  ],
  [
    "GitHub authorization expired. Try connecting again.",
    m.github_authorization_expired_try_connecting_again,
  ],
  [
    "GitHub authorization was cancelled. You can try again.",
    m.github_authorization_was_cancelled_you_can_try_again,
  ],
  [
    "GitHub denied access. Check organization permissions or SSO.",
    m.github_denied_access_check_organization_permissions_or_sso,
  ],
  [
    "GitHub denied access. Check your account or organization permissions, then try again.",
    m.github_denied_access_check_your_account_or_organization_permissio,
  ],
  [
    "GitHub denied access. Check your organization permissions or SSO.",
    m.github_denied_access_check_your_organization_permissions_or_sso,
  ],
  [
    "GitHub did not provide a verified primary email. Verify your email on GitHub, then try again.",
    m.github_did_not_provide_a_verified_primary_email_verify_your_email,
  ],
  [
    "GitHub did not return a verified primary email. Verify it, then reconnect.",
    m.github_did_not_return_a_verified_primary_email_verify_it_then_rec,
  ],
  [
    "GitHub did not return a verified primary email. Verify your GitHub email, then reconnect.",
    m.github_did_not_return_a_verified_primary_email_verify_your_github,
  ],
  [
    "GitHub is temporarily rate limiting requests. Try signing in again later.",
    m.github_is_temporarily_rate_limiting_requests_try_signing_in_again,
  ],
  [
    "GitHub rate limit reached. Try again later.",
    m.github_rate_limit_reached_try_again_later,
  ],
  ["GitHub repository access connected.", m.github_repository_access_connected],
  [
    "GitHub repository access disconnected. Existing projects remain available.",
    m.github_repository_access_disconnected_existing_projects_remain_av,
  ],
  [
    "GitHub repository access expired. Reconnect GitHub.",
    m.github_repository_access_expired_reconnect_github,
  ],
  [
    "GitHub sign-in was cancelled. You can try again when you are ready.",
    m.github_sign_in_was_cancelled_you_can_try_again_when_you_are_ready,
  ],
  [
    "Import could not be confirmed. Check the project list before trying again.",
    m.import_could_not_be_confirmed_check_the_project_list_before_tryin,
  ],
  [
    "Import recovered. Return to your workspace to retry it.",
    m.import_recovered_return_to_your_workspace_to_retry_it,
  ],
  [
    "Import recovery could not be confirmed. Check its status and try again.",
    m.import_recovery_could_not_be_confirmed_check_its_status_and_try_a,
  ],
  [
    "Import status could not be checked. Check projects again.",
    m.import_status_could_not_be_checked_check_projects_again,
  ],
  [
    "Model access checked. You can return to your workspace.",
    m.model_access_checked_you_can_return_to_your_workspace,
  ],
  [
    "Model access recovery could not be confirmed. Check its status in Recovery options.",
    m.model_access_recovery_could_not_be_confirmed_check_its_status_in,
  ],
  [
    "More imports need recovery before returning to your workspace.",
    m.more_imports_need_recovery_before_returning_to_your_workspace,
  ],
  ["Opening your terminal…", m.opening_your_terminal],
  ["Pending", m.pending],
  ["Project and its chats deleted.", m.project_and_its_chats_deleted],
  ["Project ready.", m.project_ready],
  [
    "Projects are unavailable. Try again.",
    m.projects_are_unavailable_try_again,
  ],
  [
    "Recovery could not be confirmed. Check its status in Recovery options.",
    m.recovery_could_not_be_confirmed_check_its_status_in_recovery_opti,
  ],
  [
    "Recovery is blocked: shutdown or a model launch is still unconfirmed. Check recovery status or contact your administrator.",
    m.recovery_is_blocked_shutdown_or_a_model_launch_is_still_unconfirm,
  ],
  [
    "Recovery status could not be confirmed. Try again before starting your workspace.",
    m.recovery_status_could_not_be_confirmed_try_again_before_starting,
  ],
  [
    "Recovery status is unavailable. Try again in Recovery options.",
    m.recovery_status_is_unavailable_try_again_in_recovery_options,
  ],
  [
    "Repository renamed since import. Verify the Git remote before pushing.",
    m.repository_renamed_since_import_verify_the_git_remote_before_push,
  ],
  ["Saving access…", m.saving_access],
  [
    "Select the code and copy it manually.",
    m.select_the_code_and_copy_it_manually,
  ],
  [
    "Sign-in is temporarily unavailable. Please try again later.",
    m.sign_in_is_temporarily_unavailable_please_try_again_later,
  ],
  [
    "Sign-in may still be closing. Check again before retrying; recovery can take up to 10 minutes.",
    m.sign_in_may_still_be_closing_check_again_before_retrying_recovery,
  ],
  ["Sign-in was cancelled.", m.sign_in_was_cancelled],
  [
    "Stop active or uncertain conversations before disconnecting.",
    m.stop_active_or_uncertain_conversations_before_disconnecting,
  ],
  [
    "Stop the project’s active or uncertain runs and complete import recovery before deleting.",
    m.stop_the_project_s_active_or_uncertain_runs_and_complete_import_r,
  ],
  ["Suspended", m.suspended],
  ["Task recovered.", m.task_recovered],
  [
    "The Codex account status could not be confirmed. Check again to retry.",
    m.the_codex_account_status_could_not_be_confirmed_check_again_to_re,
  ],
  [
    "The Codex response could not be verified. Check the status before trying again.",
    m.the_codex_response_could_not_be_verified_check_the_status_before,
  ],
  [
    "The GitHub account did not match your Factory account.",
    m.the_github_account_did_not_match_your_factory_account,
  ],
  [
    "The GitHub account does not match this Factory account.",
    m.the_github_account_does_not_match_this_factory_account,
  ],
  [
    "The access change could not be confirmed. Refresh the list before trying again.",
    m.the_access_change_could_not_be_confirmed_refresh_the_list_before,
  ],
  [
    "The code could not be confirmed. Cancel and start again.",
    m.the_code_could_not_be_confirmed_cancel_and_start_again,
  ],
  [
    "The connection expired before it could open. Reconnect to try again.",
    m.the_connection_expired_before_it_could_open_reconnect_to_try_agai,
  ],
  [
    "The connection is too slow to send more input. Reconnect; unsent keystrokes will not be replayed.",
    m.the_connection_is_too_slow_to_send_more_input_reconnect_unsent_ke,
  ],
  [
    "The conversation changed or is still starting. Check its status before trying again.",
    m.the_conversation_changed_or_is_still_starting_check_its_status_be,
  ],
  [
    "The conversation could not be updated. Check its status before trying again.",
    m.the_conversation_could_not_be_updated_check_its_status_before_try,
  ],
  [
    "The display could not keep up. Your command keeps running; reconnect to see its output.",
    m.the_display_could_not_keep_up_your_command_keeps_running_reconnec,
  ],
  [
    "The environment changed. Try again to check its current status.",
    m.the_environment_changed_try_again_to_check_its_current_status,
  ],
  [
    "The environment configuration needs administrator attention.",
    m.the_environment_configuration_needs_administrator_attention,
  ],
  [
    "The environment could not be verified. Contact your administrator.",
    m.the_environment_could_not_be_verified_contact_your_administrator,
  ],
  [
    "The environment operation timed out. Try again to check its current status.",
    m.the_environment_operation_timed_out_try_again_to_check_its_curren,
  ],
  [
    "The environment service is unavailable. Try again shortly.",
    m.the_environment_service_is_unavailable_try_again_shortly,
  ],
  [
    "The environment status could not be confirmed. Try again.",
    m.the_environment_status_could_not_be_confirmed_try_again,
  ],
  [
    "The operation timed out. Its result is unknown; check the status again.",
    m.the_operation_timed_out_its_result_is_unknown_check_the_status_ag,
  ],
  [
    "The original GitHub address changed or is inaccessible. Check the Git remote before pushing.",
    m.the_original_github_address_changed_or_is_inaccessible_check_the,
  ],
  [
    "The previous answer may have been sent. Check its status before changing it.",
    m.the_previous_answer_may_have_been_sent_check_its_status_before_ch,
  ],
  [
    "The previous terminal attachment could not be confirmed closed. Try an explicit takeover; contact your administrator if this continues.",
    m.the_previous_terminal_attachment_could_not_be_confirmed_closed_tr,
  ],
  [
    "The project or environment changed. Check its status and try again.",
    m.the_project_or_environment_changed_check_its_status_and_try_again,
  ],
  [
    "The run may still be active. Stop the environment from your profile and recover the run before continuing.",
    m.the_run_may_still_be_active_stop_the_environment_from_your_profil,
  ],
  [
    "The run was interrupted. You can start another message.",
    m.the_run_was_interrupted_you_can_start_another_message,
  ],
  [
    "The shell exited. Reconnect to open a shell again.",
    m.the_shell_exited_reconnect_to_open_a_shell_again,
  ],
  [
    "The sign-in code has expired. Check the status before starting again.",
    m.the_sign_in_code_has_expired_check_the_status_before_starting_aga,
  ],
  [
    "The terminal connection closed. Reconnect to continue.",
    m.the_terminal_connection_closed_reconnect_to_continue,
  ],
  [
    "The terminal connection failed. Check your environment, then reconnect.",
    m.the_terminal_connection_failed_check_your_environment_then_reconn,
  ],
  [
    "The terminal connection received an invalid message. Reconnect to try again.",
    m.the_terminal_connection_received_an_invalid_message_reconnect_to,
  ],
  [
    "The terminal did not open in time. Check your environment, then reconnect.",
    m.the_terminal_did_not_open_in_time_check_your_environment_then_rec,
  ],
  [
    "The terminal is unavailable. Check that your environment is running, then reconnect.",
    m.the_terminal_is_unavailable_check_that_your_environment_is_runnin,
  ],
  [
    "This account changed or cannot be edited. The list has been refreshed; review it before trying again.",
    m.this_account_changed_or_cannot_be_edited_the_list_has_been_refres,
  ],
  [
    "This conversation is too long to transfer. Start a new chat to use another provider.",
    m.this_conversation_is_too_long_to_transfer_start_a_new_chat_to_use,
  ],
  [
    "This environment uses an API key or another unsupported method. Connect your personal ChatGPT/Codex account instead.",
    m.this_environment_uses_an_api_key_or_another_unsupported_method_co,
  ],
  [
    "This paste is too large. Paste fewer than 64 KiB at a time.",
    m.this_paste_is_too_large_paste_fewer_than_64_kib_at_a_time,
  ],
  [
    "This sign-in attempt expired or could not be verified. Start a new sign-in below.",
    m.this_sign_in_attempt_expired_or_could_not_be_verified_start_a_new,
  ],
  [
    "This terminal is open in another tab, or the environment changed. You can explicitly take over the terminal.",
    m.this_terminal_is_open_in_another_tab_or_the_environment_changed_y,
  ],
  [
    "Too many requests. Wait a moment, then check again.",
    m.too_many_requests_wait_a_moment_then_check_again,
  ],
  [
    "Too many requests. Wait a moment, then try again.",
    m.too_many_requests_wait_a_moment_then_try_again,
  ],
  [
    "Your access has been suspended. Contact your administrator before trying again.",
    m.your_access_has_been_suspended_contact_your_administrator_before,
  ],
  [
    "Your access request is waiting for administrator approval. Sign in again after your access is approved.",
    m.your_access_request_is_waiting_for_administrator_approval_sign_in,
  ],
  [
    "Your saved storage could not be found. Contact your administrator; it will not be replaced automatically.",
    m.your_saved_storage_could_not_be_found_contact_your_administrator,
  ],
  [
    "Your session changed. Reconnect after your access has been checked.",
    m.your_session_changed_reconnect_after_your_access_has_been_checked,
  ],
  [
    "Your session is no longer active. Sign in again to reconnect.",
    m.your_session_is_no_longer_active_sign_in_again_to_reconnect,
  ],
  ["active", m.active_96879611],
  ["admin", m.admin],
  ["high", m.high],
  ["low", m.low],
  ["medium", m.medium],
  ["member", m.member],
  ["minimal", m.minimal],
  ["none", m.none],
  ["pending", m.pending_62a2fed3],
  ["requests", m.requests],
  ["suspended", m.suspended_de2d423a],
  ["xhigh", m.xhigh],
])

export function translateMessage(message: string): string {
  return messages.get(message)?.() ?? message
}
