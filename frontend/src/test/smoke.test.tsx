import { render, screen } from '@testing-library/react'

describe('frontend test environment', () => {
  it('renders React into jsdom', () => {
    render(<main>配电网仿真工作台</main>)

    expect(screen.getByRole('main')).toHaveTextContent('配电网仿真工作台')
  })
})
