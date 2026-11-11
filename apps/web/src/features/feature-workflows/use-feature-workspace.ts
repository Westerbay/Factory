import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import {
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query"
import * as m from "@workspace/i18n/messages"

import { ApiFailure } from "../../lib/api/client"
import { useSession } from "../../lib/auth/session-context"
import { useLocale } from "../../lib/i18n"
import * as api from "./api"
import {
  createOperationRegistry,
  hasActiveAttempts,
  hasUncertainAttempts,
} from "./state"

function problem(error: unknown): string {
  if (error instanceof api.FeatureFailure) {
    switch (error.code) {
      case "feature-conflict":
        return m.feature_conflict()
      case "feature-repository-authorization-required":
        return m.github_repository_access_expired_reconnect_github()
      case "feature-execution-uncertain":
        return m.feature_execution_uncertain()
      case "feature-stale-snapshot":
        return m.feature_stale_snapshot()
      case "feature-review-blocked":
        return m.feature_review_blocked()
    }
  }
  return m.feature_command_error()
}

export function useFeatureWorkspace({
  available,
  featureId,
  projectId,
}: {
  available: boolean
  featureId?: string
  projectId?: string
}) {
  const locale = useLocale()
  const { state: session, controller } = useSession()
  const client = useQueryClient()
  const scope =
    session.status === "active"
      ? `${session.session.user.id}:${controller.getScope()}`
      : ""
  const csrf = session.status === "active" ? session.session.csrfToken : ""
  const registry = useMemo(createOperationRegistry, [])
  const lifetime = useRef<AbortController | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>()
  const enabled = available && Boolean(scope)
  const listKey = ["features", scope, "list", projectId ?? ""] as const
  const detailKey = ["features", scope, "detail", featureId ?? ""] as const
  const diffKey = ["features", scope, "diff", featureId ?? ""] as const
  const features = useInfiniteQuery({
    queryKey: listKey,
    queryFn: ({ pageParam, signal }) =>
      api.list({ projectId, before: pageParam }, signal),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => page.nextCursor ?? undefined,
    enabled,
    retry: 1,
    refetchInterval: 15_000,
    refetchIntervalInBackground: false,
  })
  const detail = useQuery({
    queryKey: detailKey,
    queryFn: ({ signal }) => api.get(featureId!, signal),
    enabled: enabled && Boolean(featureId),
    retry: 1,
    refetchInterval: (query) => {
      const feature = query.state.data
      return feature &&
        (hasActiveAttempts(feature) ||
          hasUncertainAttempts(feature) ||
          (feature.feature.stage === "architecture" && !feature.architecture) ||
          feature.feature.stage === "execution" ||
          feature.feature.stage === "review")
        ? 2000
        : 10_000
    },
    refetchIntervalInBackground: false,
  })
  const diff = useQuery({
    queryKey: [
      ...diffKey,
      detail.data?.review?.snapshotId ?? "",
      detail.data?.delivery?.status ?? "",
      detail.data?.delivery?.requestId ?? "",
    ],
    queryFn: ({ signal }) => api.diff(featureId!, signal),
    enabled: enabled && Boolean(featureId && detail.data?.review),
    retry: 1,
  })

  useEffect(() => {
    const abort = new AbortController()
    lifetime.current = abort
    return () => {
      abort.abort()
      lifetime.current = null
      registry.clear()
    }
  }, [scope, registry])
  useEffect(() => {
    setError(undefined)
  }, [featureId])
  useEffect(() => {
    if (!available) void client.cancelQueries({ queryKey: ["features", scope] })
  }, [available, client, scope])

  const refresh = useCallback(async () => {
    await client.invalidateQueries({
      queryKey: ["features", scope, "detail", featureId ?? ""],
    })
    await client.invalidateQueries({
      queryKey: ["features", scope, "diff", featureId ?? ""],
    })
    await client.invalidateQueries({ queryKey: ["features", scope, "list"] })
  }, [client, scope, featureId])

  async function command(
    key: string,
    action: (operationId: string, signal: AbortSignal) => Promise<api.Feature>,
    persistedId?: string
  ) {
    const signal = lifetime.current?.signal
    if (
      busy ||
      !enabled ||
      !csrf ||
      !signal ||
      signal.aborted ||
      controller.getSnapshot().status !== "active"
    )
      return
    const operationId = persistedId ?? registry.get(scope, key)
    setBusy(true)
    setError(undefined)
    try {
      const value = await action(operationId, signal)
      if (signal.aborted) return
      registry.complete(scope, key, operationId)
      client.setQueryData(
        ["features", scope, "detail", value.feature.id],
        value
      )
      await client.invalidateQueries({ queryKey: ["features", scope, "list"] })
      await client.invalidateQueries({
        queryKey: ["features", scope, "diff", value.feature.id],
      })
      return value
    } catch (reason) {
      if (!signal.aborted) {
        setError(reason)
        if (
          reason instanceof ApiFailure &&
          (reason.status === 401 || reason.status === 403)
        )
          controller.authorizationFailed()
        else await refresh()
      }
    } finally {
      if (!signal.aborted) setBusy(false)
    }
  }

  const feature = detail.data
  return {
    feature,
    features: features.data?.pages.flatMap((page) => page.items) ?? [],
    loading: features.isPending,
    listError: features.isError,
    loadingFeature: detail.isPending,
    featureError: detail.isError,
    busy,
    message: error ? problem(error) : "",
    needsRepositoryReconnect:
      error instanceof api.FeatureFailure &&
      error.code === "feature-repository-authorization-required",
    hasMore: features.hasNextPage,
    loadingMore: features.isFetchingNextPage,
    loadMore: () => void features.fetchNextPage(),
    retryList: () => void features.refetch(),
    refresh,
    diff: diff.data,
    diffLoading: diff.isFetching,
    diffError: diff.isError,
    reloadDiff: () => void diff.refetch(),
    create: (project: string, provider: api.FeatureProvider) =>
      command(
        `create:${project}:${provider}:${locale}`,
        (operationId, signal) =>
          api.create(
            {
              operationId,
              projectId: project,
              provider,
              title: m.new_feature(),
              locale,
            },
            csrf,
            signal
          )
      ),
    approve: (revision: { id: string; digest: string }) =>
      featureId &&
      command(
        `approve:${featureId}:${revision.id}:${revision.digest}`,
        (operationId, signal) =>
          api.approve(
            featureId,
            { operationId, revisionId: revision.id, digest: revision.digest },
            csrf,
            signal
          )
      ),
    reopen: (revision: { id: string; digest: string }) =>
      featureId &&
      command(
        `reopen:${featureId}:${revision.id}:${revision.digest}`,
        (operationId, signal) =>
          api.reopen(
            featureId,
            { operationId, revisionId: revision.id, digest: revision.digest },
            csrf,
            signal
          )
      ),
    resume: (attemptId?: string) =>
      featureId &&
      command(
        `resume:${featureId}:${attemptId ?? "step"}`,
        (operationId, signal) =>
          api.resume(featureId, { operationId, attemptId }, csrf, signal)
      ),
    review: (snapshotId: string) =>
      featureId &&
      command(`review:${featureId}:${snapshotId}`, (operationId, signal) =>
        api.review(featureId, { operationId, snapshotId }, csrf, signal)
      ),
    draft: (snapshotId: string) =>
      featureId &&
      command(
        `draft:${featureId}:${snapshotId}`,
        (operationId, signal) =>
          api.draft(featureId, { operationId, snapshotId }, csrf, signal),
        feature?.delivery?.snapshotId === snapshotId &&
          ["claimed", "uncertain"].includes(feature.delivery.status)
          ? feature.delivery.requestId
          : undefined
      ),
  }
}

export type FeatureWorkspaceState = ReturnType<typeof useFeatureWorkspace>
