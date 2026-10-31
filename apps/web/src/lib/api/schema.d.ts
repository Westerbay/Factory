export interface paths {
  "/api/admin/users": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get: operations["listUsers"]
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/admin/users/{id}/access": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch: operations["changeAccess"]
    trace?: never
  }
  "/api/agent-connections/codex": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get: operations["getCodexConnection"]
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/agent-connections/codex/login": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    post: operations["beginCodexLogin"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/agent-connections/codex/login/cancel": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    post: operations["cancelCodexLogin"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/agent-connections/codex/logout": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    post: operations["logoutCodex"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/auth/github/callback": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get: operations["completeGithubLogin"]
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/auth/github/start": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    /** @description An explicit Accept: text/html with nonzero quality returns start failures to the fixed frontend sign-in page. Other clients retain JSON errors and HTTP status. Retry-After is preserved for rate limits. */
    get: operations["startGithubLogin"]
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/auth/logout": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    post: operations["logout"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/conversations": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get: operations["listConversations"]
    put?: never
    post: operations["createConversation"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/conversations/model-probe/recover": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    post: operations["recoverModelProbe"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/conversations/models": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get: operations["listConversationModels"]
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/conversations/recoverable": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get: operations["listRecoverableRuns"]
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/conversations/{id}": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get: operations["getConversation"]
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/conversations/{id}/runs": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    post: operations["startConversationRun"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/conversations/{id}/runs/{run}/cancel": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    post: operations["cancelConversationRun"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/conversations/{id}/runs/{run}/events": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get: operations["streamConversationRun"]
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/conversations/{id}/runs/{run}/questions/{request}": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    post: operations["answerConversationQuestion"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/conversations/{id}/runs/{run}/recover": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    post: operations["recoverConversationRun"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/environment": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    /** @description Read only the current user's environment. Does not create or start resources. */
    get: operations["getEnvironment"]
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/environment/initialize": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    /** @description Create a running intent only if the current user has no environment; existing intent is never changed. */
    post: operations["initializeEnvironment"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/environment/start": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    /** @description Persist a persistent running intent for the current user. Opposite intents are queued by generation. */
    post: operations["startEnvironment"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/environment/stop": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    /** @description Persist a stopped intent; no volume is removed. */
    post: operations["stopEnvironment"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/session": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get: operations["getSession"]
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/terminal/attachments": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get?: never
    put?: never
    post: operations["createTerminalAttachment"]
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/api/terminal/socket": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    /** @description Upgrade with exact Origin and factory.terminal.v1 subprotocol. Within five seconds send TerminalClientMessage.authenticate containing the single-use ticket. Further JSON controls use TerminalClientMessage; data output is binary and controls use TerminalServerMessage. No credentials in URL or subprotocol. */
    get: operations["connectTerminal"]
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/health/live": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get: operations["healthLive"]
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
  "/health/ready": {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    get: operations["healthReady"]
    put?: never
    post?: never
    delete?: never
    options?: never
    head?: never
    patch?: never
    trace?: never
  }
}
export type webhooks = Record<string, never>
export interface components {
  schemas: {
    /** @enum {string} */
    AccessAction: "active" | "suspended"
    AccessBody: {
      /** Format: int64 */
      expectedVersion: number
      status: components["schemas"]["AccessAction"]
    }
    AdminUser: {
      displayName: string
      handle: string
      /** Format: uuid */
      id: string
      provider: string
      role: components["schemas"]["UserRole"]
      status: components["schemas"]["UserStatus"]
      /** Format: int64 */
      version: number
    }
    AgentAccount: {
      email?: string | null
      plan: string
    }
    AgentConnectionResponse: {
      account?: components["schemas"]["AgentAccount"] | null
      challenge?: components["schemas"]["AgentLoginChallenge"] | null
      /** @description Bounded application error code, never the raw provider response. */
      errorCode?: string | null
      /** Format: date-time */
      observedAt?: string | null
      provider: components["schemas"]["AgentProvider"]
      status: components["schemas"]["AgentConnectionStatus"]
    }
    /** @enum {string} */
    AgentConnectionStatus:
      | "unavailable"
      | "disconnected"
      | "connecting"
      | "connected"
      | "error"
    /** @description Device authorization instructions, never provider credentials or a provider login ID. */
    AgentLoginChallenge: {
      /** Format: date-time */
      expiresAt: string
      /** Format: uuid */
      operationId: string
      userCode: string
      verificationUrl: string
    }
    /** @enum {string} */
    AgentProvider: "codex"
    AnswerBody: {
      answers: components["schemas"]["AnswerView"][]
      /** Format: uuid */
      operationId: string
    }
    AnswerView: {
      /** Format: uuid */
      id: string
      value: string
    }
    ApiError: {
      code: string
      message: string
      /** Format: uuid */
      requestId: string
    }
    AttachmentRequest: {
      takeOver: boolean
    }
    AttachmentResponse: {
      /** Format: date-time */
      expiresAt: string
      ticket: string
    }
    CancelAgentLoginBody: {
      /** Format: uuid */
      operationId: string
    }
    ConversationPageView: {
      items: components["schemas"]["ConversationView"][]
      /** Format: uuid */
      nextCursor?: string | null
    }
    ConversationView: {
      createdAt: string
      /** Format: uuid */
      id: string
    }
    EnvironmentResponse: {
      /** @description Bounded public error code; never includes provider responses or resource names. */
      errorCode?: string | null
      /** Format: date-time */
      observedAt?: string | null
      status: components["schemas"]["EnvironmentStatus"]
    }
    /** @enum {string} */
    EnvironmentStatus:
      | "not_created"
      | "starting"
      | "running"
      | "stopping"
      | "stopped"
      | "error"
      | "unavailable"
    EventView: {
      kind: string
      /** Format: uuid */
      runId: string
      text?: string | null
    }
    HistoryView: {
      conversation: components["schemas"]["ConversationView"]
      messages: components["schemas"]["MessageView"][]
      messagesTruncated: boolean
      requests: components["schemas"]["RequestView"][]
      requestsTruncated: boolean
      runs: components["schemas"]["RunView"][]
      runsTruncated: boolean
    }
    Live: {
      /** @enum {string} */
      status: "ok"
    }
    MessageView: {
      content: string
      createdAt: string
      /** Format: uuid */
      id: string
      role: string
      /** Format: uuid */
      runId: string
    }
    ModelView: {
      id: string
      name: string
    }
    QuestionOptionView: {
      description: string
      label: string
    }
    QuestionView: {
      allowFreeText: boolean
      header: string
      /** Format: uuid */
      id: string
      options: components["schemas"]["QuestionOptionView"][]
      question: string
    }
    Ready: {
      code: string
      status: string
    }
    /** @description Minimal stopped-state recovery identifier; never includes conversation contents. */
    RecoverableRunView: {
      /** Format: uuid */
      conversationId: string
      launchSettled: boolean
      /** Format: uuid */
      runId: string
    }
    /** @enum {string} */
    RequestStateView: "pending" | "claimed" | "uncertain"
    RequestView: {
      /** Format: uuid */
      id: string
      kind: string
      questions: components["schemas"]["QuestionView"][]
      /** Format: uuid */
      runId: string
      state: components["schemas"]["RequestStateView"]
    }
    RunView: {
      /** Format: uuid */
      conversationId: string
      /** Format: uuid */
      id: string
      model: string
      /** Format: uuid */
      operationId: string
      status: string
    }
    SessionResponse: {
      csrfToken: string
      /** Format: date-time */
      expiresAt: string
      user: components["schemas"]["SessionUser"]
    }
    SessionUser: {
      displayName: string
      handle: string
      /** Format: uuid */
      id: string
      role: components["schemas"]["UserRole"]
    }
    StartBody: {
      input: string
      model: string
      /** Format: uuid */
      operationId: string
    }
    /** @description UTF-8 JSON inputs for factory.terminal.v1. Output data uses binary frames. */
    TerminalClientMessage:
      | {
          ticket: string
          /** @enum {string} */
          type: "authenticate"
        }
      | {
          data: string
          /** @enum {string} */
          type: "input"
        }
      | {
          /** Format: int32 */
          cols: number
          /** Format: int32 */
          rows: number
          /** @enum {string} */
          type: "resize"
        }
      | {
          /** Format: int32 */
          bytes: number
          /** @enum {string} */
          type: "ack"
        }
    /** @enum {string} */
    TerminalErrorCode:
      | "unauthorized"
      | "unavailable"
      | "conflict"
      | "invalid_ticket"
      | "invalid_input"
      | "slow_client"
      | "cleanup_uncertain"
    TerminalServerMessage:
      | {
          /** @enum {string} */
          type: "ready"
        }
      | {
          code: components["schemas"]["TerminalErrorCode"]
          /** @enum {string} */
          type: "detached"
        }
      | {
          /** @enum {string} */
          type: "exit"
        }
      | {
          code: components["schemas"]["TerminalErrorCode"]
          /** @enum {string} */
          type: "error"
        }
    /** @enum {string} */
    UserRole: "member" | "admin"
    /** @enum {string} */
    UserStatus: "pending" | "active" | "suspended"
    UsersResponse: {
      items: components["schemas"]["AdminUser"][]
      /** Format: uuid */
      nextCursor?: string | null
    }
  }
  responses: never
  parameters: never
  requestBodies: never
  headers: never
  pathItems: never
}
export type $defs = Record<string, never>
export interface operations {
  listUsers: {
    parameters: {
      query: {
        status: components["schemas"]["UserStatus"]
        cursor?: string
        limit?: number
      }
      header?: never
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Users in stable UUID order */
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["UsersResponse"]
        }
      }
      /** @description Invalid input */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Access denied */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description User not found */
      404: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Conflict or protected administrator */
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Dependency unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  changeAccess: {
    parameters: {
      query?: never
      header: {
        /** @description Session-bound CSRF value; exact frontend Origin also required. */
        "X-CSRF-Token": string
      }
      path: {
        id: string
      }
      cookie?: never
    }
    requestBody: {
      content: {
        "application/json": components["schemas"]["AccessBody"]
      }
    }
    responses: {
      /** @description Updated member access */
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["AdminUser"]
        }
      }
      /** @description Invalid input */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Access denied */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description User not found */
      404: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Conflict or protected administrator */
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Dependency unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  getCodexConnection: {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Current account or unavailable state; pending codes belong only to their initiating session */
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["AgentConnectionResponse"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Operation owned by another session or environment changed */
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Identity unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  beginCodexLogin: {
    parameters: {
      query?: never
      header: {
        /** @description Session CSRF token; exact frontend Origin required; empty request body */
        "X-CSRF-Token": string
      }
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Device authorization started; may already be connected */
      202: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["AgentConnectionResponse"]
        }
      }
      /** @description Body must be empty */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Origin or CSRF rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Already pending, expired or environment changed */
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Unsupported authentication method */
      422: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Invalid provider response or login failure */
      502: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Provider unavailable or cleanup uncertain */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  cancelCodexLogin: {
    parameters: {
      query?: never
      header: {
        /** @description Session CSRF token; exact frontend Origin required */
        "X-CSRF-Token": string
      }
      path?: never
      cookie?: never
    }
    requestBody: {
      content: {
        "application/json": components["schemas"]["CancelAgentLoginBody"]
      }
    }
    responses: {
      /** @description Operation cancellation confirmed */
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["AgentConnectionResponse"]
        }
      }
      /** @description Invalid body */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Origin or CSRF rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Operation differs or environment changed */
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Unsupported authentication method */
      422: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Invalid provider response or login failure */
      502: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Provider unavailable or cleanup uncertain */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  logoutCodex: {
    parameters: {
      query?: never
      header: {
        /** @description Session CSRF token; exact frontend Origin required; empty request body */
        "X-CSRF-Token": string
      }
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Provider logout confirmed */
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["AgentConnectionResponse"]
        }
      }
      /** @description Body must be empty */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Origin or CSRF rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Operation owned by another session or environment changed */
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Unsupported authentication method */
      422: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Invalid provider response or login failure */
      502: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Provider unavailable or cleanup uncertain */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  completeGithubLogin: {
    parameters: {
      query?: {
        state?: string
        code?: string
        error?: string
      }
      header?: never
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Fixed frontend redirect; issues session only for admitted user */
      302: {
        headers: {
          [name: string]: unknown
        }
        content?: never
      }
      /** @description Invalid input */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Dependency unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  startGithubLogin: {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Redirect to GitHub with state and PKCE, or browser recovery at the fixed frontend sign-in page */
      302: {
        headers: {
          [name: string]: unknown
        }
        content?: never
      }
      /** @description Invalid input */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Dependency unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  logout: {
    parameters: {
      query?: never
      header: {
        /** @description Session-bound CSRF value; exact frontend Origin also required. */
        "X-CSRF-Token": string
      }
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Session revoked and cookie cleared */
      204: {
        headers: {
          [name: string]: unknown
        }
        content?: never
      }
      /** @description Invalid input */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Origin or CSRF denied */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Dependency unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  listConversations: {
    parameters: {
      query?: {
        before?: string
      }
      header?: never
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ConversationPageView"]
        }
      }
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      404: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  createConversation: {
    parameters: {
      query?: never
      header: {
        /** @description Exact configured frontend Origin required */
        Origin: string
        /** @description Session CSRF token required */
        "X-CSRF-Token": string
      }
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ConversationView"]
        }
      }
      /** @description Invalid path or body */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Exact Origin or CSRF rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Request rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Service unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  recoverModelProbe: {
    parameters: {
      query?: never
      header: {
        /** @description Exact configured frontend Origin required */
        Origin: string
        /** @description Session CSRF token required */
        "X-CSRF-Token": string
      }
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      204: {
        headers: {
          [name: string]: unknown
        }
        content?: never
      }
      /** @description Invalid path or body */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Exact Origin or CSRF rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Request rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Service unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  listConversationModels: {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ModelView"][]
        }
      }
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  listRecoverableRuns: {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["RecoverableRunView"][]
        }
      }
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  getConversation: {
    parameters: {
      query?: never
      header?: never
      path: {
        id: string
      }
      cookie?: never
    }
    requestBody?: never
    responses: {
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["HistoryView"]
        }
      }
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      404: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  startConversationRun: {
    parameters: {
      query?: never
      header: {
        /** @description Exact configured frontend Origin required */
        Origin: string
        /** @description Session CSRF token required */
        "X-CSRF-Token": string
      }
      path: {
        id: string
      }
      cookie?: never
    }
    requestBody: {
      content: {
        "application/json": components["schemas"]["StartBody"]
      }
    }
    responses: {
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["RunView"]
        }
      }
      /** @description Invalid path or body */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Exact Origin or CSRF rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      404: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Request rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Service unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  cancelConversationRun: {
    parameters: {
      query?: never
      header: {
        /** @description Exact configured frontend Origin required */
        Origin: string
        /** @description Session CSRF token required */
        "X-CSRF-Token": string
      }
      path: {
        id: string
        run: string
      }
      cookie?: never
    }
    requestBody?: never
    responses: {
      204: {
        headers: {
          [name: string]: unknown
        }
        content?: never
      }
      /** @description Invalid path or body */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Exact Origin or CSRF rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      404: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Request rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Service unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  streamConversationRun: {
    parameters: {
      query?: never
      header: {
        /** @description Exact configured frontend Origin required */
        Origin: string
      }
      path: {
        id: string
        run: string
      }
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Authorized bounded event stream */
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "text/event-stream": unknown
        }
      }
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      404: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  answerConversationQuestion: {
    parameters: {
      query?: never
      header: {
        /** @description Exact configured frontend Origin required */
        Origin: string
        /** @description Session CSRF token required */
        "X-CSRF-Token": string
      }
      path: {
        id: string
        run: string
        request: string
      }
      cookie?: never
    }
    requestBody: {
      content: {
        "application/json": components["schemas"]["AnswerBody"]
      }
    }
    responses: {
      204: {
        headers: {
          [name: string]: unknown
        }
        content?: never
      }
      /** @description Invalid path or body */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Exact Origin or CSRF rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      404: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Request rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Service unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  recoverConversationRun: {
    parameters: {
      query?: never
      header: {
        /** @description Exact configured frontend Origin required */
        Origin: string
        /** @description Session CSRF token required */
        "X-CSRF-Token": string
      }
      path: {
        id: string
        run: string
      }
      cookie?: never
    }
    requestBody?: never
    responses: {
      204: {
        headers: {
          [name: string]: unknown
        }
        content?: never
      }
      /** @description Invalid path or body */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Exact Origin or CSRF rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Request rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Service unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  getEnvironment: {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Environment state, including unavailable when not prepared */
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["EnvironmentResponse"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Identity or persistence unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  initializeEnvironment: {
    parameters: {
      query?: never
      header: {
        /** @description Session CSRF token. Exact frontend Origin and an empty body are required. */
        "X-CSRF-Token": string
      }
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Current environment intent */
      202: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["EnvironmentResponse"]
        }
      }
      /** @description Body must be empty */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Origin or CSRF rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Environment service unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  startEnvironment: {
    parameters: {
      query?: never
      header: {
        /** @description Session CSRF token. Exact frontend Origin and an empty body are required. */
        "X-CSRF-Token": string
      }
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Running intent accepted */
      202: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["EnvironmentResponse"]
        }
      }
      /** @description Body must be empty */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Origin or CSRF rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Conflicting intent */
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Environment service unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  stopEnvironment: {
    parameters: {
      query?: never
      header: {
        /** @description Session CSRF token. Exact frontend Origin and an empty body are required. */
        "X-CSRF-Token": string
      }
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Stopped intent accepted */
      202: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["EnvironmentResponse"]
        }
      }
      /** @description Body must be empty */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Origin or CSRF rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Conflicting intent */
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Environment service unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  getSession: {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Current active session */
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["SessionResponse"]
        }
      }
      /** @description Invalid input */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Access denied */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description User not found */
      404: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Conflict or protected administrator */
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Dependency unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  createTerminalAttachment: {
    parameters: {
      query?: never
      header: {
        /** @description Session CSRF token; exact frontend Origin required */
        "X-CSRF-Token": string
      }
      path?: never
      cookie?: never
    }
    requestBody: {
      content: {
        "application/json": components["schemas"]["AttachmentRequest"]
      }
    }
    responses: {
      /** @description Single-use terminal ticket, expires within 30 seconds */
      201: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["AttachmentResponse"]
        }
      }
      /** @description Invalid body */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Origin or CSRF rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Already attached or environment changed */
      409: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Terminal unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  connectTerminal: {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Authenticated WebSocket upgrade; ticket still required before exec */
      101: {
        headers: {
          [name: string]: unknown
        }
        content?: never
      }
      /** @description Invalid upgrade or subprotocol */
      400: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Active session required */
      401: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Origin rejected */
      403: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Rate limit */
      429: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
      /** @description Terminal unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["ApiError"]
        }
      }
    }
  }
  healthLive: {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Process is alive */
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["Live"]
        }
      }
    }
  }
  healthReady: {
    parameters: {
      query?: never
      header?: never
      path?: never
      cookie?: never
    }
    requestBody?: never
    responses: {
      /** @description Required services ready */
      200: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["Ready"]
        }
      }
      /** @description Required services unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content: {
          "application/json": components["schemas"]["Ready"]
        }
      }
    }
  }
}
