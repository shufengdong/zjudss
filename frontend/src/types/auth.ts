export interface User {
  id: string
  name: string
}

export type AuthState =
  | { status: 'loading' }
  | { status: 'anonymous' }
  | { status: 'authenticated'; user: User; epoch: number }
