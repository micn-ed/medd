// Runs before every test file (vite.config.ts's test.setupFiles). Mocks the one Tauri API the
// render pipeline calls directly (`convertFileSrc`, for image resolution) so golden-file tests
// exercise the real production code path rather than a hand-rolled stand-in.
import { mockConvertFileSrc } from '@tauri-apps/api/mocks'
// Extends `expect` with DOM matchers (`toBeInTheDocument`, etc.) for component tests — added
// alongside @testing-library/svelte (increment-7 QA finding 3: no test could previously
// distinguish "state was set" from "the user can see it").
import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/svelte'
import { afterEach } from 'vitest'

mockConvertFileSrc('macos')

// Unmounts whatever a component test rendered and clears jsdom's body — without this, every
// `render()` in a later test would pile onto the previous one's DOM, and a query like
// `getByRole('alert')` would find multiple elements instead of failing (or passing) for the one
// test actually mounted it.
afterEach(cleanup)
