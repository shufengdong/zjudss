import { describe, expect, it } from 'vitest'
import { authReducer, initialAuthState, preferenceKey } from './auth'

describe('auth state', () => {
  it('moves from loading to anonymous or authenticated and replaces user state by epoch', () => {
    expect(authReducer(initialAuthState, { type: 'anonymous' })).toEqual({ status: 'anonymous' })
    const alice = authReducer(initialAuthState, {
      type: 'authenticated',
      user: { id: 'alice', name: 'Alice' },
    })
    expect(alice).toEqual({
      status: 'authenticated',
      user: { id: 'alice', name: 'Alice' },
      epoch: 1,
    })
    expect(authReducer(alice, {
      type: 'authenticated',
      user: { id: 'bob', name: 'Bob' },
    })).toEqual({
      status: 'authenticated',
      user: { id: 'bob', name: 'Bob' },
      epoch: 2,
    })
    expect(authReducer(alice, { type: 'anonymous' })).toEqual({ status: 'anonymous' })
  })

  it('keys navigation preferences by fixed user ID', () => {
    expect(preferenceKey('alice')).toBe('zjudss:last-session:alice')
  })
})
