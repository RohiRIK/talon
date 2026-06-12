import { createContext, useContext } from "react"
import type { Role } from "./types"

/**
 * Criterion 7: the viewer role hides/disables every mutating affordance.
 * Provided by App after /me validates the token; defaults to viewer so an
 * unknown state never shows admin controls.
 */
export const RoleContext = createContext<Role>("viewer")

export function useRole(): Role {
  return useContext(RoleContext)
}
