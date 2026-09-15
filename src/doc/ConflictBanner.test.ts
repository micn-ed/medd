// The first Svelte component test in the project (increment-7 QA finding 3): every prior test
// asserted on module state or on rendered HTML strings, never on a mounted component. That gap is
// not hypothetical — the architect's finding 4 is a tab whose `detached` state is set by a tested
// code path and rendered by nothing at all, and a state-only suite is structurally incapable of
// noticing it. This mounts the real component and asserts what's actually on screen.
import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, expect, test, vi } from 'vitest'
import ConflictBanner from './ConflictBanner.svelte'

describe('ConflictBanner', () => {
  test('renders the headline and the consequence of each choice', () => {
    render(ConflictBanner, { props: { onReload: () => {}, onKeepMine: () => {} } })

    expect(screen.getByText('This file changed on disk.')).toBeInTheDocument()
    // The only safeguard around "Keep mine" while Diff… is deferred (increment-7 review, §9) is
    // this sentence actually being on screen -- assert its substance, not just its presence.
    expect(screen.getByText(/reloading discards your unsaved edits/i)).toBeInTheDocument()
    expect(screen.getByText(/keeping yours overwrites the version on disk/i)).toBeInTheDocument()
  })

  test('Reload calls onReload, not onKeepMine', async () => {
    const onReload = vi.fn()
    const onKeepMine = vi.fn()
    render(ConflictBanner, { props: { onReload, onKeepMine } })

    await fireEvent.click(screen.getByRole('button', { name: 'Reload' }))

    expect(onReload).toHaveBeenCalledTimes(1)
    expect(onKeepMine).not.toHaveBeenCalled()
  })

  test('Keep mine calls onKeepMine, not onReload', async () => {
    const onReload = vi.fn()
    const onKeepMine = vi.fn()
    render(ConflictBanner, { props: { onReload, onKeepMine } })

    await fireEvent.click(screen.getByRole('button', { name: 'Keep mine' }))

    expect(onKeepMine).toHaveBeenCalledTimes(1)
    expect(onReload).not.toHaveBeenCalled()
  })

  test('is announced as an alert, so it is not silent to assistive technology either', () => {
    render(ConflictBanner, { props: { onReload: () => {}, onKeepMine: () => {} } })

    expect(screen.getByRole('alert')).toBeInTheDocument()
  })
})
