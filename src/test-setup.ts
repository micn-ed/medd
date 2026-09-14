// Runs before every test file (vite.config.ts's test.setupFiles). Mocks the one Tauri API the
// render pipeline calls directly (`convertFileSrc`, for image resolution) so golden-file tests
// exercise the real production code path rather than a hand-rolled stand-in.
import { mockConvertFileSrc } from '@tauri-apps/api/mocks'

mockConvertFileSrc('macos')
