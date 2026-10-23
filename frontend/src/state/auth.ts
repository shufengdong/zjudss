import type { AuthState, User } from '../types'

export const AUTHENTICATION_REQUIRED_EVENT = 'zjudss:authentication-required'
export const initialAuthState: AuthState = { status: 'loading' }

export type AuthAction =
  | { type: 'anonymous' }
  | { type: 'authenticated'; user: User }

export function authReducer(state: AuthState, action: AuthAction): AuthState {
  if (action.type === 'anonymous') return { status: 'anonymous' }
  return {
    status: 'authenticated',
    user: action.user,
    epoch: state.status === 'authenticated' ? state.epoch + 1 : 1,
  }
}

export const preferenceKey = (userId: string) => `zjudss:last-session:${userId}`

let notificationQueued = false

export function emitAuthenticationRequired() {
  if (notificationQueued) return
  notificationQueued = true
  window.dispatchEvent(new Event(AUTHENTICATION_REQUIRED_EVENT))
  window.setTimeout(() => {
    notificationQueued = false
  }, 0)
}
