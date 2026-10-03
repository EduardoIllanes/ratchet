// Scenario tests of the context-meter module (openspec/specs/context-meter/spec.md).
// One test per scenario, named exactly as the scenario; run by `claude plugin test`.
import type { ContextCategory, On, SessionContextBreakdown } from 'claude-code'
import { expect, mock, test } from 'claude-code/testing'
import type { Engine } from 'claude-code/testing'

const WINDOW = 200000

type Agent = { id: string; description: string; type: string; status: string }
type Rig = {
  statuses: (string | undefined)[]
  toasts: string[]
  commands: string[]
  usageCalls: unknown[]
  usage: { tokens?: number; percent?: number; window: number }
  breakdown: SessionContextBreakdown | undefined
  usageFails: boolean
  agents: Agent[]
  files: string[]
  dirs: string[]
}

const fileStat = { kind: 'file', size: 1, mtimeMs: 0, isLink: false } as const
const dirStat = { kind: 'dir', size: 0, mtimeMs: 0, isLink: false } as const

// Stands for the engine beneath the module and records what the module does to it.
function rig(on: On, o: { files?: string[]; dirs?: string[] } = {}): Rig {
  const r: Rig = {
    statuses: [],
    toasts: [],
    commands: [],
    usageCalls: [],
    usage: { window: WINDOW },
    breakdown: undefined,
    usageFails: false,
    agents: [],
    files: o.files ?? ['/work/repo/ratchet.toml'],
    dirs: o.dirs ?? [],
  }
  mock.clock(on)
  on('session.start', (_$, e) => ({ cwd: e.cwd }))
  on('fs.stat', (_$, e) => {
    if (r.files.includes(e.path)) return { value: fileStat }
    if (r.dirs.includes(e.path)) return { value: dirStat }
    return { deny: 'ENOENT' }
  })
  on('command.register', (_$, e) => {
    r.commands.push(e.name)
    return { value: undefined } as never
  })
  on('session.usage', (_$, e) => {
    r.usageCalls.push(e)
    if (r.usageFails) return { deny: 'usage unavailable' }
    const breakdown = e?.breakdown ? r.breakdown : undefined
    return {
      value: {
        startedAt: 0,
        context: { ...r.usage, ...(breakdown ? { breakdown } : {}) },
        rateLimits: [],
      },
    }
  })
  on('agent.list', () => ({ value: r.agents }))
  on('ui.status', (_$, e) => {
    r.statuses.push(e.text)
    return { value: undefined }
  })
  on('ui.toast', (_$, e) => {
    r.toasts.push(e.text)
    return { value: undefined }
  })
  on('session.measure', (_$, e) => ({ changed: e.changed }))
  on('turn.step', async function* () {
    return {
      turnId: 't',
      index: 0,
      answer: '',
      toolUses: [],
      stopReason: 'tool_use',
      usage: {
        input_tokens: 2000,
        output_tokens: 500,
        cache_read_input_tokens: 40000,
        cache_creation_input_tokens: 8000,
        model: 'claude-sonnet-5-5',
      },
    }
  })
  return r
}

const start = ($: Engine, cwd = '/work/repo') =>
  $.session.start({ cwd, surface: 'terminal', isInteractive: true })

const measure = (
  $: Engine,
  r: Rig,
  tokens: number | undefined,
  percent: number | undefined,
  window = WINDOW,
  changed: ('context' | 'cost')[] = ['context'],
) => {
  r.usage = tokens === undefined ? { window } : { tokens, percent, window }
  return $.session.measure({
    context: r.usage,
    rateLimits: [],
    changed,
  })
}

async function request($: Engine, agentId?: string) {
  const s = $.turn.step({
    turnId: 't',
    index: 0,
    model: 'claude-sonnet-5-5',
    messageCount: 3,
    ...(agentId ? { agentId } : {}),
  })
  // Read the stream to its end by hand: the done value is the step's result as the engine
  // got it (`s.result` resolved undefined in 2.1.288 when the stream was read this way).
  let n = await s.next()
  while (!n.done) n = await s.next()
  await s.result
  return n.value
}

const last = (r: Rig) => r.statuses[r.statuses.length - 1]

const MAIN = 'ctx ████░░░░░░ 42% · 84k/200k'

const cat = (name: string, tokens: number, kind: ContextCategory['kind']): ContextCategory => ({
  name,
  tokens,
  color: 'promptBorder',
  isDeferred: kind === 'deferred',
  kind,
})

const BREAKDOWN: SessionContextBreakdown = {
  categories: [
    cat('System prompt', 6000, 'used'),
    cat('Messages', 80000, 'used'),
    cat('Deferred tools', 3000, 'deferred'),
    cat('Free space', 114000, 'free'),
  ],
  totalTokens: 86000,
  maxTokens: 200000,
  rawMaxTokens: 200000,
  autocompactSource: 'model-default',
  percentage: 43,
  gridRows: [],
  model: 'claude-sonnet-5-5',
  memoryFiles: [],
  mcpTools: [],
  agents: [],
  autoCompactThreshold: 167000,
  isAutoCompactEnabled: true,
  apiUsage: null,
}

const SURFACES = ['terminal', 'desktop'] as const
const mountPane = ($: Engine, surface: (typeof SURFACES)[number]) =>
  $.ui.mount({
    plugin: 'ratchet',
    surface,
    component: 'Pane',
    requestId: 'context-meter',
    props: {
      title: 'Context',
      isFocused: false,
      bodyColumns: 40,
      placement: 'dock',
      scroll: { offset: 0, bodyRows: 20 },
      view: {},
    },
  })

const AGENT1: Agent = {
  id: 'agent-1',
  description: 'Read the big file',
  type: 'ratchet:reader',
  status: 'running',
}

test('Without ratchet.toml the meter stays silent', async ($, on) => {
  const r = rig(on, { files: [] })
  await start($, '/work/plain/src')
  await measure($, r, 180000, 90)
  expect(r.statuses).toEqual([])
  expect(r.toasts).toEqual([])
  expect(r.commands).toEqual([])
  // Control: the rig does see an opted-in session, so the silence above is the module's.
  r.files = ['/work/repo/ratchet.toml']
  await start($, '/work/repo')
  expect(r.commands.length).toBeGreaterThan(0)
})

test('A ratchet.toml in an ancestor opts the session in', async ($, on) => {
  const r = rig(on, { files: ['/work/repo/ratchet.toml'] })
  await start($, '/work/repo/crates/core')
  expect(r.commands).toEqual(['ctx'])
  expect(r.statuses.length).toBeGreaterThan(0)
  expect(typeof last(r)).toBe('string')
})

test('A directory named ratchet.toml does not opt in', async ($, on) => {
  const r = rig(on, { files: [], dirs: ['/work/repo/ratchet.toml'] })
  await start($, '/work/repo')
  expect(r.commands).toEqual([])
  expect(r.statuses).toEqual([])
  // Control: the same rig with a file does opt in.
  r.dirs = []
  r.files = ['/work/repo/ratchet.toml']
  await start($, '/work/repo')
  expect(r.commands).toEqual(['ctx'])
})

test('A measurement pins the fill on the status line', async ($, on) => {
  const r = rig(on)
  await start($)
  await measure($, r, 84000, 42)
  expect(last(r)).toBe(MAIN)
})

test('Before the first response the line says there is no reading', async ($, on) => {
  const r = rig(on)
  await start($)
  expect(last(r)).toBe('ctx ░░░░░░░░░░ –/200k')
})

test('A main-loop request moves the line within a turn', async ($, on) => {
  const r = rig(on)
  await start($)
  await request($)
  expect(last(r)).toBe('ctx ███░░░░░░░ 25% · 50k/200k')
})

test('A measurement that moved only the cost leaves the line alone', async ($, on) => {
  const r = rig(on)
  await start($)
  await measure($, r, 84000, 42)
  expect(last(r)).toBe(MAIN)
  const pinned = r.statuses.length
  await measure($, r, 84000, 42, WINDOW, ['cost'])
  expect(r.statuses.length).toBe(pinned)
})

test('A million-token window is written in M', async ($, on) => {
  const r = rig(on)
  await start($)
  await measure($, r, 250000, 25, 1000000)
  expect(last(r)).toBe('ctx ███░░░░░░░ 25% · 250k/1M')
})

test('Each threshold toasts once, and a drop re-arms it', async ($, on) => {
  const r = rig(on)
  await start($)
  for (const p of [50, 72, 75, 88, 90]) await measure($, r, p * 2000, p)
  await measure($, r, undefined, undefined)
  for (const p of [20, 71]) await measure($, r, p * 2000, p)
  expect(r.toasts).toHaveLength(3)
  for (const [i, pct] of ['72%', '88%', '71%'].entries()) {
    expect(r.toasts[i] ?? '').toContain(pct)
    expect(r.toasts[i] ?? '').toContain('/compact')
    expect(r.toasts[i] ?? '').toContain('ratchet task handoff')
  }
})

test("A running subagent's requests tail the status line", async ($, on) => {
  const r = rig(on)
  r.agents = [AGENT1]
  await start($)
  await measure($, r, 84000, 42)
  await request($, 'agent-1')
  expect(last(r)).toBe(`${MAIN} · 1 agent (max 50k)`)
})

test('A finished subagent leaves the status line', async ($, on) => {
  const r = rig(on)
  r.agents = [{ ...AGENT1, status: 'completed' }]
  await start($)
  await measure($, r, 84000, 42)
  await request($, 'agent-1')
  expect(last(r)).toBe(MAIN)
})

test('The pane draws the breakdown on every surface', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  await start($)
  await measure($, r, 86000, 43)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: /43% · 86k \/ 200k/ })).toBeDefined()
    expect(await ui.find({ text: /Messages.*80k.*40\.0%/ })).toBeDefined()
    expect(await ui.find({ text: /Deferred tools/ })).toBeUndefined()
    expect(await ui.find({ text: /auto-compact at 167k/ })).toBeDefined()
    await ui.unmount()
  }
  const asked = r.usageCalls.filter(c => (c as { breakdown?: string } | undefined)?.breakdown)
  expect(asked.length).toBeGreaterThan(0)
  for (const c of asked) expect((c as { breakdown?: string }).breakdown).toBe('summary')
})

test('The pane lists subagents with their fill', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.agents = [AGENT1]
  await start($)
  await request($, 'agent-1')
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(
      await ui.find({
        text: /ratchet:reader · 50k ~25% · peak 50k · 1 req · Read the big file/,
      }),
    ).toBeDefined()
    await ui.unmount()
  }
})

test('The pane says so before the first response', async ($, on) => {
  const r = rig(on)
  r.breakdown = undefined
  await start($)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: /No breakdown yet/ })).toBeDefined()
    await ui.unmount()
  }
})

test('A failing reading never fails a model request', async ($, on) => {
  const r = rig(on)
  await start($)
  // Control: with a working reading the module does pin the line on a request.
  await request($)
  expect(last(r)).toBe('ctx ███░░░░░░░ 25% · 50k/200k')
  r.usageFails = true
  const result = await request($)
  expect(result).toEqual({
    turnId: 't',
    index: 0,
    answer: '',
    toolUses: [],
    stopReason: 'tool_use',
    usage: {
      input_tokens: 2000,
      output_tokens: 500,
      cache_read_input_tokens: 40000,
      cache_creation_input_tokens: 8000,
      model: 'claude-sonnet-5-5',
    },
  })
})
