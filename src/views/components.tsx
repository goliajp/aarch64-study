import {
  Alert,
  Badge,
  Button,
  Card,
  Input,
  Progress,
  Spinner,
  StatusBadge,
  Tabs,
} from '@goliapkg/gds'
import { useState } from 'react'

export function ComponentsView() {
  const [activeTab, setActiveTab] = useState('display')

  return (
    <div className="space-y-8">
      <div>
        <h1
          className="text-fg text-2xl font-bold"
          style={{ textShadow: '0 0 20px var(--gds-accent, #3b82f6)' }}
        >
          Components
        </h1>
        <p className="text-fg-muted mt-1">GDS component showcase with live examples.</p>
      </div>

      <Tabs
        onChange={setActiveTab}
        tabs={[
          { id: 'display', label: 'Display' },
          { id: 'input', label: 'Input' },
          { id: 'feedback', label: 'Feedback' },
        ]}
        active={activeTab}
      />

      {activeTab === 'display' && (
        <div className="space-y-6">
          <Section title="Badge">
            <div className="flex flex-wrap gap-2">
              <Badge>Default</Badge>
              <Badge color="info">Info</Badge>
              <Badge color="success">Success</Badge>
              <Badge color="warning">Warning</Badge>
              <Badge color="danger">Danger</Badge>
            </div>
          </Section>

          <Section title="StatusBadge">
            <div className="flex flex-wrap gap-3">
              <StatusBadge label="Healthy" status="active" />
              <StatusBadge label="Degraded" status="warning" />
              <StatusBadge label="Down" status="error" />
            </div>
          </Section>

          <Section title="Card">
            <div className="grid grid-cols-2 gap-4">
              <Card>
                <div className="p-4">
                  <h3 className="text-fg text-sm font-semibold">Default Card</h3>
                  <p className="text-fg-muted mt-1 text-xs">A simple card container.</p>
                </div>
              </Card>
              <Card>
                <div className="p-4">
                  <h3 className="text-fg text-sm font-semibold">Another Card</h3>
                  <p className="text-fg-muted mt-1 text-xs">Cards compose well together.</p>
                </div>
              </Card>
            </div>
          </Section>

          <Section title="Progress">
            <div className="space-y-3">
              <Progress value={25} />
              <Progress value={60} />
              <Progress value={90} />
            </div>
          </Section>
        </div>
      )}

      {activeTab === 'input' && (
        <div className="space-y-6">
          <Section title="Button">
            <div className="flex flex-wrap gap-2">
              <Button size="sm">Primary</Button>
              <Button size="sm" variant="secondary">
                Outline
              </Button>
              <Button size="sm" variant="ghost">
                Ghost
              </Button>
              <Button disabled size="sm">
                Disabled
              </Button>
            </div>
          </Section>

          <Section title="Input">
            <div className="max-w-sm space-y-3">
              <Input placeholder="Default input..." />
              <Input disabled placeholder="Disabled input..." />
            </div>
          </Section>
        </div>
      )}

      {activeTab === 'feedback' && (
        <div className="space-y-6">
          <Section title="Alert">
            <div className="space-y-3">
              <Alert>This is a default alert message.</Alert>
              <Alert variant="warning">Warning: check your configuration.</Alert>
              <Alert variant="danger">Error: something went wrong.</Alert>
            </div>
          </Section>

          <Section title="Spinner">
            <div className="flex items-center gap-4">
              <Spinner size="sm" />
              <Spinner size="default" />
              <Spinner size="lg" />
            </div>
          </Section>
        </div>
      )}
    </div>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <h2 className="text-fg mb-3 text-sm font-semibold">{title}</h2>
      {children}
    </div>
  )
}
