import { Badge, Card, GlassButton, GlassCard, Spinner } from '@goliapkg/gds'
import { Link } from 'react-router'

import { useGithubRepo } from '../api/github'

function v(pkg: string): string {
  return (__DEP_VERSIONS__[pkg] ?? '').replace(/^[\^~>=<]*/g, '').split('.')[0]
}

const FEATURES = [
  {
    title: `React ${v('react')}`,
    description: 'Concurrent rendering, server components, and the latest React APIs.',
    pkg: 'react',
  },
  {
    title: `React Router ${v('react-router')}`,
    description: 'URL as single source of truth. Path params, query params, nested layouts.',
    pkg: 'react-router',
  },
  {
    title: `React Query ${v('@tanstack/react-query')}`,
    description: 'Server state management with caching, refetching, and stale-while-revalidate.',
    pkg: '@tanstack/react-query',
  },
  {
    title: 'GDS',
    description: 'GOLIA Design System with theme-aware tokens, dark mode, and 70+ components.',
    pkg: '@goliapkg/gds',
  },
  {
    title: `Tailwind CSS ${v('tailwindcss')}`,
    description: 'Utility-first CSS with design tokens and zero-config content detection.',
    pkg: 'tailwindcss',
  },
  {
    title: 'Jotai',
    description: 'Primitive and flexible state management for ephemeral client state.',
    pkg: 'jotai',
  },
  {
    title: `Vite ${v('vite')}`,
    description: 'Instant dev server, fast HMR, and optimized production builds.',
    pkg: 'vite',
  },
  {
    title: `Vitest ${v('vitest')}`,
    description: 'Unit testing with native ESM, TypeScript, and jsdom support.',
    pkg: 'vitest',
  },
  {
    title: `TypeScript ${v('typescript')}`,
    description: 'Strict mode, bundler resolution, latest language features.',
    pkg: 'typescript',
  },
]

export function HomeView() {
  return (
    <div className="space-y-8">
      <div className="space-y-3">
        <div className="flex items-center gap-3">
          <h1
            className="text-fg text-2xl font-bold"
            style={{ textShadow: '0 0 20px var(--gds-accent, #3b82f6)' }}
          >
            GOLIA Web Starter
          </h1>
          <Badge color="info">v{__APP_VERSION__}</Badge>
        </div>
        <p className="text-fg-muted max-w-2xl text-base">
          Production-ready boilerplate for GOLIA web applications. Includes pre-configured
          toolchain, design system integration, and best-practice patterns.
        </p>
      </div>

      <div className="flex gap-3">
        <Link to="/components">
          <GlassButton size="sm" variant="accent">
            View Components
          </GlassButton>
        </Link>
        <Link to="/state">
          <GlassButton size="sm">State Demo</GlassButton>
        </Link>
      </div>

      <RepoCard owner="goliajp" name="gds" />

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {FEATURES.map((f) => (
          <GlassCard className="glow-card" key={f.pkg}>
            <div className="p-4">
              <h3 className="text-fg mb-1 text-sm font-semibold">{f.title}</h3>
              <p className="text-fg-muted text-xs leading-relaxed">{f.description}</p>
            </div>
          </GlassCard>
        ))}
      </div>
    </div>
  )
}

function RepoCard({ owner, name }: { owner: string; name: string }) {
  const { data, isLoading, error } = useGithubRepo(owner, name)

  return (
    <Card>
      <div className="p-4">
        <div className="text-fg-muted mb-2 text-[10px] font-semibold tracking-wider uppercase">
          Live Data — useQuery + Axios
        </div>
        {isLoading && (
          <div className="flex items-center gap-2">
            <Spinner size="sm" />
            <span className="text-fg-muted text-xs">Fetching GitHub API...</span>
          </div>
        )}
        {error && <p className="text-xs text-red-400">Failed to fetch: {error.message}</p>}
        {data && (
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <a
                className="text-fg text-sm font-semibold hover:underline"
                href={`https://github.com/${data.full_name}`}
                rel="noopener noreferrer"
                target="_blank"
              >
                {data.full_name}
              </a>
              <Badge>{data.language}</Badge>
            </div>
            <p className="text-fg-muted text-xs">{data.description}</p>
            <div className="text-fg-muted flex gap-4 text-xs">
              <span>&#9733; {data.stargazers_count}</span>
              <span>Forks {data.forks_count}</span>
              <span>Issues {data.open_issues_count}</span>
              <span>Updated {new Date(data.updated_at).toLocaleDateString()}</span>
            </div>
          </div>
        )}
      </div>
    </Card>
  )
}
