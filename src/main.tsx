import './index.css'

import { loadPersistedTheme, resolveThemeCssVars } from '@goliapkg/gds/systems'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { createBrowserRouter, Navigate, RouterProvider } from 'react-router'

import { AppLayout } from './app'
import { AboutView } from './views/about'
import { CpuView } from './views/cpu'

// Pre-render the persisted theme so the user doesn't see a flash of the
// wrong palette before React mounts.
const saved = loadPersistedTheme()
if (saved) {
  const mode =
    saved.mode === 'system'
      ? window.matchMedia('(prefers-color-scheme: dark)').matches
        ? 'dark'
        : 'light'
      : saved.mode
  const vars = resolveThemeCssVars(saved, mode as 'dark' | 'light')
  const root = document.documentElement
  for (const [k, v] of Object.entries(vars)) root.style.setProperty(k, v as string)
  root.dataset.theme = mode
}

// In production the app is mounted under labs.golia.jp/aarch64; vite injects
// that as `import.meta.env.BASE_URL`. Drop the trailing slash for React Router.
const ROUTER_BASE =
  import.meta.env.BASE_URL && import.meta.env.BASE_URL !== '/'
    ? import.meta.env.BASE_URL.replace(/\/$/, '')
    : undefined

const router = createBrowserRouter(
  [
    {
      children: [
        { element: <CpuView />, index: true },
        { element: <AboutView />, path: 'about' },
        { element: <Navigate replace to="/" />, path: '*' },
      ],
      element: <AppLayout />,
      path: '/',
    },
  ],
  ROUTER_BASE ? { basename: ROUTER_BASE } : undefined
)

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <RouterProvider router={router} />
  </StrictMode>
)
