import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { validateToken } from "./api"

// criterion 7: a bad token must never be persisted; validation happens
// against /api/v1/me with the candidate token, not the stored one.

const localStorageMock = (() => {
  let store: Record<string, string> = {}
  return {
    getItem: (k: string) => store[k] ?? null,
    setItem: (k: string, v: string) => {
      store[k] = v
    },
    removeItem: (k: string) => {
      delete store[k]
    },
    clear: () => {
      store = {}
    },
  }
})()

beforeEach(() => {
  vi.stubGlobal("localStorage", localStorageMock)
  localStorageMock.clear()
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe("validateToken", () => {
  it("returns identity for a valid token and sends the candidate as bearer", async () => {
    const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
      const auth = (init?.headers as Record<string, string>).authorization
      expect(auth).toBe("Bearer candidate-token")
      return new Response(JSON.stringify({ name: "ops", role: "admin" }), {
        status: 200,
      })
    })
    vi.stubGlobal("fetch", fetchMock)

    const me = await validateToken("candidate-token")
    expect(me).toEqual({ name: "ops", role: "admin" })
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/v1/me",
      expect.objectContaining({}),
    )
  })

  it("rejects on 401 and never touches localStorage", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("{}", { status: 401 })),
    )

    await expect(validateToken("bad-token")).rejects.toThrow("invalid token")
    expect(localStorageMock.getItem("talon_api_token")).toBeNull()
  })
})
