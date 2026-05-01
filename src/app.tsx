import { useFonts } from '@goliapkg/gds'
import { useSetThemeDensity, useSetThemeMotion, useThemeEffect } from '@goliapkg/gds/systems'
import { useEffect } from 'react'
import { Link, Outlet, useLocation } from 'react-router'

import { ThemeToggle } from './components/theme-toggle'

const NAV = [
  { label: 'CPU', path: '/' },
  { label: 'About', path: '/about' },
]

export function AppLayout() {
  useThemeEffect()
  useFonts()

  const setDensity = useSetThemeDensity()
  const setMotion = useSetThemeMotion()
  useEffect(() => {
    setDensity('default')
    setMotion('full')
  }, [setDensity, setMotion])

  const location = useLocation()

  const isActive = (path: string) =>
    path === '/' ? location.pathname === '/' : location.pathname.startsWith(path)

  return (
    <div className="flex h-full flex-col">
      <header className="border-border bg-bg/80 flex h-12 shrink-0 items-center justify-between gap-2 border-b px-3 backdrop-blur-xl sm:px-6">
        <div className="flex min-w-0 items-center gap-3 sm:gap-6">
          <Link className="text-fg flex shrink-0 items-center gap-2 text-sm font-semibold" to="/">
            <img
              alt="GOLIA"
              className="h-5 w-5 rounded-sm"
              src="https://cdn.golia.jp/logo-icon.png"
              style={{ filter: 'drop-shadow(0 0 6px var(--gds-accent, #3b82f6))' }}
            />
            <span className="hidden sm:inline">AArch64 Study</span>
            <span className="sm:hidden">aarch64</span>
          </Link>
          <nav className="flex items-center gap-1">
            {NAV.map((item) => (
              <Link
                className={`rounded-md px-2 py-1.5 text-sm transition-colors sm:px-3 ${
                  isActive(item.path)
                    ? 'bg-accent/10 text-accent'
                    : 'text-fg-muted hover:bg-bg-tertiary hover:text-fg'
                }`}
                key={item.path}
                to={item.path}
              >
                {item.label}
              </Link>
            ))}
          </nav>
        </div>
        <ThemeToggle />
      </header>

      <main className="flex-1 overflow-y-auto">
        <div className="px-3 py-4 sm:px-6 sm:py-6 2xl:px-10">
          <Outlet />
        </div>
      </main>

      <footer className="border-border text-fg-muted flex h-8 shrink-0 items-center justify-center border-t bg-white/5 text-xs backdrop-blur-sm">
        AArch64 Study &middot; simulator (Rust → WASM)
      </footer>
    </div>
  )
}
