import { Card } from '@goliapkg/gds'

type StackItem = { name: string; pkg: string; url: string }

const STACK: StackItem[] = [
  { name: 'React', pkg: 'react', url: 'https://react.dev' },
  { name: 'React Router', pkg: 'react-router', url: 'https://reactrouter.com' },
  { name: 'Jotai', pkg: 'jotai', url: 'https://jotai.org' },
  { name: 'Tailwind CSS', pkg: 'tailwindcss', url: 'https://tailwindcss.com' },
  { name: 'GDS', pkg: '@goliapkg/gds', url: 'https://github.com/goliajp/gds' },
  { name: 'TypeScript', pkg: 'typescript', url: 'https://typescriptlang.org' },
  { name: 'Vite', pkg: 'vite', url: 'https://vite.dev' },
  { name: 'Vitest', pkg: 'vitest', url: 'https://vitest.dev' },
  { name: 'ESLint', pkg: 'eslint', url: 'https://eslint.org' },
  { name: 'Prettier', pkg: 'prettier', url: 'https://prettier.io' },
]

function resolveVersion(spec: string): string {
  return spec.replace(/^[\^~>=<]*/g, '')
}

export function AboutView() {
  return (
    <div className="space-y-8">
      <div>
        <h1
          className="text-fg text-2xl font-bold"
          style={{ textShadow: '0 0 20px var(--gds-accent, #3b82f6)' }}
        >
          About
        </h1>
        <p className="text-fg-muted mt-1">Stack versions and project structure.</p>
      </div>

      <div>
        <h2 className="text-fg mb-3 text-sm font-semibold">Tech Stack</h2>
        <Card>
          <div className="divide-border divide-y">
            {STACK.map((s) => (
              <div className="flex items-center justify-between px-4 py-2.5" key={s.pkg}>
                <a
                  className="text-fg text-sm hover:underline"
                  href={s.url}
                  rel="noopener noreferrer"
                  target="_blank"
                >
                  {s.name}
                </a>
                <span className="text-fg-muted font-mono text-xs">
                  {resolveVersion(__DEP_VERSIONS__[s.pkg] ?? '?')}
                </span>
              </div>
            ))}
          </div>
        </Card>
      </div>

      <div>
        <h2 className="text-fg mb-3 text-sm font-semibold">Project Structure</h2>
        <Card>
          <pre className="text-fg-muted overflow-x-auto p-4 font-mono text-xs leading-relaxed">
            {`web/
├── src/
│   ├── components/     # shared components
│   ├── views/          # route components
│   ├── app.tsx          # layout shell
│   ├── main.tsx         # entry point + router
│   └── index.css        # tailwind + gds tokens
├── eslint.config.mjs
├── prettier.config.mjs
├── tsconfig.json
├── vite.config.ts
└── package.json`}
          </pre>
        </Card>
      </div>

      <div>
        <h2 className="text-fg mb-3 text-sm font-semibold">Quick Start</h2>
        <Card>
          <pre className="text-fg-muted overflow-x-auto p-4 font-mono text-xs leading-relaxed">
            {`bun install          # install deps
bun run dev          # dev server
bun run build        # production build
bun run test         # run tests
bun run check        # typecheck + lint + format check`}
          </pre>
        </Card>
      </div>
    </div>
  )
}
