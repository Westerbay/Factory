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
    ApiError: {
      code: string
      message: string
      /** Format: uuid */
      requestId: string
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
          "application/json": {
            /** @constant */
            status: "ok"
          }
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
        content?: never
      }
      /** @description Required services unavailable */
      503: {
        headers: {
          [name: string]: unknown
        }
        content?: never
      }
    }
  }
}
