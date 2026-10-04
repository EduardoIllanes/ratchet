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
  stepTokens?: number
  agentsFail: boolean
  registerFails: boolean
  messages: unknown[]
  usageStdout?: string
  reads: Record<string, string>
  messagesFail: boolean
  tasks: { id: string; title: string; status: string }[]
  usageJson: unknown
  usageExit: number
  runs: string[][]
  windows: boolean
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
    agentsFail: false,
    registerFails: false,
    messages: [],
    reads: {},
    messagesFail: false,
    tasks: [],
    usageJson: { tasks: [] },
    usageExit: 0,
    runs: [],
    windows: false,
  }
  mock.clock(on)
  on('session.start', (_$, e) => ({ cwd: e.cwd }))
  on('fs.stat', (_$, e) => {
    if (r.windows && e.path.endsWith('/bin/ratchet.exe')) return { value: fileStat }
    if (r.files.includes(e.path)) return { value: fileStat }
    if (r.dirs.includes(e.path)) return { value: dirStat }
    return { deny: 'ENOENT' }
  })
  on('command.register', (_$, e) => {
    if (r.registerFails) return { deny: 'register unavailable' }
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
  on('agent.list', () => (r.agentsFail ? { deny: 'agents unavailable' } : { value: r.agents }))
  on('session.id', () => ({ value: 'sess-1' }))
  // The module reads the Messages API form: `{ as: 'api' }`.
  on('session.messages', () =>
    r.messagesFail ? { deny: 'messages unavailable' } : { value: r.messages as never },
  )
  on('fs.read', (_$, e) => {
    const text = r.reads[e.path]
    return text === undefined ? { deny: 'ENOENT' } : { value: text }
  })
  on('session.end', (_$, e) => ({ sessionId: e.sessionId }))
  on('ui.open', () => ({ value: { isPlaced: true } }))
  on('process.run', (_$, e) => {
    r.runs.push([...e.argv])
    const done = (exitCode: number, stdout: string) => ({
      value: { exitCode, stdout, stderr: '', isStdoutTruncated: false, isStderrTruncated: false },
    })
    if (e.argv.includes('usage')) return done(r.usageExit, r.usageStdout ?? JSON.stringify(r.usageJson))
    return done(0, JSON.stringify(r.tasks))
  })
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
        input_tokens: r.stepTokens ?? 2000,
        output_tokens: 500,
        cache_read_input_tokens: r.stepTokens === undefined ? 40000 : 0,
        cache_creation_input_tokens: r.stepTokens === undefined ? 8000 : 0,
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


type Node = { type?: string; props?: Record<string, unknown>; children?: unknown[] }
const textOf = (n: unknown): string =>
  typeof n === 'string' ? n : ((n as Node).children ?? []).map(textOf).join('')

// The props of the bar of the line whose numbers match: the deepest element showing them,
// whose first element child is the bar (or the element itself when one Text draws the line).
function barProps(tree: unknown, numbers: RegExp): Record<string, unknown> | undefined {
  const line = new RegExp(`^\\s*[█░]+\\s*${numbers.source}`)
  let best: Node | undefined
  const walk = (n: unknown) => {
    if (typeof n === 'string') return
    const node = n as Node
    if (line.test(textOf(node))) best = node
    for (const c of node.children ?? []) walk(c)
  }
  walk(tree)
  if (!best) return undefined
  const bar = (best.children ?? []).find(c => typeof c !== 'string' && /^[█░]+$/.test(textOf(c)))
  return ((bar as Node | undefined) ?? best).props
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
const IMPLEMENTER: Agent = { ...AGENT1, type: 'ratchet:implementer' }

declare const setTimeout: (fn: () => void, ms: number) => unknown
// The plugin under test is the repo root; the mocks' own `$.plugin.root` is a temporary test plugin.
const ROOT = decodeURIComponent(new URL('../../../', (import.meta as unknown as { url: string }).url).pathname).replace(/\/$/, '')
const norm = (path: string | undefined) =>
  (path ?? '').replace(/\\/g, '/').replace(/^\/(?=[A-Za-z]:)/, '').toLowerCase()
const argvNorm = (argv: string[]) => [norm(argv[0]), ...argv.slice(1)]
const settle = () => new Promise<void>(done => setTimeout(done, 25))

const ctxCommand = async ($: Engine) => {
  await $.command.run({
    command: 'ctx',
    args: '',
    origin: { kind: 'composer' },
    presentation: { isFullscreen: false, columns: 80 },
  })
  await settle()
}

const sess = (reason: 'clear' | 'other') => ({
  reason,
  sessionId: 'sess-1',
  resume: { id: 'sess-1' },
})

const endSession = ($: Engine, reason: 'clear' | 'other' = 'clear') => $.session.end(sess(reason))

const node = (n: unknown): Node => n as Node
const all = (tree: unknown): Node[] => {
  const out: Node[] = []
  const walk = (n: unknown) => {
    if (typeof n === 'string') return
    out.push(node(n))
    for (const c of node(n).children ?? []) walk(c)
  }
  walk(tree)
  return out
}
// The deepest element whose text matches (none of its children's does as a whole).
const deepest = (tree: unknown, re: RegExp): Node | undefined => {
  const hits = all(tree).filter(n => re.test(textOf(n)))
  return hits.find(n => !(n.children ?? []).some(c => typeof c !== 'string' && re.test(textOf(c))))
}
// The colour of the first element child of the row that begins `<mark> <name>`.
const markColor = (tree: unknown, name: string) => {
  const rowNode = deepest(tree, new RegExp(`^\\S\\s*${name}`))
  const mark = (rowNode?.children ?? []).find(c => typeof c !== 'string') as Node | undefined
  return mark?.props?.color
}
const tokensOf = (bucket: Record<string, number>, role: string) => ({
  role,
  model: 'm',
  session: 's',
  tokens: { input: 0, cache_write: 0, cache_read: 0, output: 0, thinking: 0, ...bucket },
})
const inProgress = (id: string) => ({ id, title: 'a task', status: 'in_progress' })

type Use = { name: string; input: Record<string, unknown>; chars: number; id: string }
const toolUse = (name: string, input: Record<string, unknown>, chars: number, i: number): Use => ({
  name,
  input,
  chars,
  id: `tu-${i}`,
})
// The Messages API form: tool_use blocks, then their tool_result blocks. Odd results carry
// their text as text blocks, even ones as a string.
const withUses = (...uses: Use[]): unknown[] => [
  {
    role: 'assistant',
    content: uses.map(u => ({ type: 'tool_use', id: u.id, name: u.name, input: u.input })),
  },
  {
    role: 'user',
    content: uses.map((u, i) => ({
      type: 'tool_result',
      tool_use_id: u.id,
      content: i % 2 === 0 ? 'x'.repeat(u.chars) : [{ type: 'text', text: 'x'.repeat(u.chars) }],
    })),
  },
]

test('Without ratchet.toml the meter stays silent', async ($, on) => {
  const r = rig(on, { files: [] })
  await start($, '/work/plain/src')
  await measure($, r, 180000, 90)
  expect(r.statuses).toEqual([])
  expect(r.toasts).toEqual([])
  expect(r.commands).toEqual([])
  expect(r.runs).toEqual([])
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

test('Counts that round to a thousand k are written in M', async ($, on) => {
  const r = rig(on)
  await start($)
  await measure($, r, 999600, 50, 2000000)
  expect(last(r)).toBe('ctx █████░░░░░ 50% · 1M/2M')
  await measure($, r, 1040000, 52, 2000000)
  expect(last(r)).toBe('ctx █████░░░░░ 52% · 1M/2M +40k')
})

test("Each turn's move follows the fill", async ($, on) => {
  const r = rig(on)
  await start($)
  await measure($, r, 84000, 42)
  await measure($, r, 96000, 48)
  expect(last(r)).toBe('ctx █████░░░░░ 48% · 96k/200k +12k')
  await measure($, r, 66000, 33)
  expect(last(r)).toBe('ctx ███░░░░░░░ 33% · 66k/200k -30k')
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
  r.agents = [IMPLEMENTER]
  await start($)
  await measure($, r, 84000, 42)
  await request($, 'agent-1')
  expect(last(r)).toBe(`${MAIN} │ implementer █░░░░ 25%`)
})

test('Past three running subagents the tail counts the rest', async ($, on) => {
  const r = rig(on)
  r.agents = [
    { ...AGENT1, id: 'agent-1', type: 'ratchet:reader' },
    { ...AGENT1, id: 'agent-2', type: 'Explore' },
    { ...AGENT1, id: 'agent-3', type: 'ratchet:implementer' },
    { ...AGENT1, id: 'agent-4', type: 'general-purpose' },
  ]
  await start($)
  await measure($, r, 84000, 42)
  for (const [id, tokens] of [
    ['agent-1', 20000],
    ['agent-2', 60000],
    ['agent-3', 160000],
    ['agent-4', 100000],
  ] as const) {
    r.stepTokens = tokens
    await request($, id)
  }
  expect(last(r)).toBe(
    `${MAIN} │ implementer ████░ 80% · general-purpose ███░░ 50% · Explore ██░░░ 30% +1`,
  )
})

test('A finished subagent leaves the status line', async ($, on) => {
  const r = rig(on)
  r.agents = [{ ...AGENT1, status: 'completed' }]
  await start($)
  await measure($, r, 84000, 42)
  await request($, 'agent-1')
  expect(last(r)).toBe(MAIN)
})

test('The agent just updated is kept when twenty share its time', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.agents = Array.from({ length: 21 }, (_, i) => ({
    ...AGENT1,
    id: `agent-${i + 1}`,
    description: `job <${i + 1}>`,
  }))
  await start($)
  r.stepTokens = 1000
  for (let i = 1; i <= 21; i++) await request($, `agent-${i}`)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    const listed = [...textOf(await ui.drawn()).matchAll(/job <(\d+)>/g)].map(m => m[1])
    expect(new Set(listed).size).toBe(20)
    expect(listed).toContain('21')
    await ui.unmount()
  }
})

test('/clear forgets the subagents and the trend', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.agents = [AGENT1]
  await start($)
  await request($, 'agent-1')
  await measure($, r, 40000, 20)
  await measure($, r, 50000, 25)
  // Control: before the end the pane has both.
  const before = await mountPane($, 'terminal')
  expect(await before.find({ text: /Subagents/ })).toBeDefined()
  expect(await before.find({ text: /Trend/ })).toBeDefined()
  await before.unmount()
  await endSession($, 'clear')
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: /Subagents/ })).toBeUndefined()
    expect(await ui.find({ text: /Trend/ })).toBeUndefined()
    await ui.unmount()
  }
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

test('The pane colours the breakdown in the Celestial palette', async ($, on) => {
  const r = rig(on)
  r.breakdown = {
    ...BREAKDOWN,
    categories: [
      cat('System prompt', 6000, 'used'),
      cat('Messages', 144000, 'used'),
      cat('Autocompact buffer', 33000, 'buffer'),
      cat('Free space', 17000, 'free'),
    ],
    totalTokens: 150000,
    percentage: 75,
  }
  await start($)
  await measure($, r, 150000, 75)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    const tree = await ui.drawn()
    expect(deepest(tree, /75%/)?.props?.color).toBe('#FAB795')
    expect(markColor(tree, 'System prompt')).toBe('#26BBD9')
    expect(markColor(tree, 'Messages')).toBe('#B877DB')
    expect(markColor(tree, 'Autocompact buffer')).toBe('#6C6F93')
    expect(markColor(tree, 'Free space')).toBe('#6C6F93')
    await ui.unmount()
  }
})

const row = (mark: string) =>
  new RegExp(`${mark}\\s*ratchet:reader[\\s\\S]*Read the big file`)

test('The pane lists subagents with their fill', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.agents = [AGENT1]
  await start($)
  await request($, 'agent-1')
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: row('●') })).toBeDefined()
    const bar = barProps(await ui.drawn(), /25% · 50k\/200k · 1 req$/)
    expect(bar?.color).toBe('#29D398')
    expect(bar?.dimColor).toBeFalsy()
    await ui.unmount()
  }
})

test('A finished subagent stays in the pane', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.agents = [AGENT1]
  await start($)
  await request($, 'agent-1')
  r.agents = []
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: row('○') })).toBeDefined()
    const bar = barProps(await ui.drawn(), /25% · 50k\/200k · 1 req$/)
    expect(bar?.dimColor).toBe(true)
    await ui.unmount()
  }
})

test('A subagent past most of its window is drawn in the hot colour with its peak', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.agents = [AGENT1]
  await start($)
  r.stepTokens = 180000
  await request($, 'agent-1')
  r.stepTokens = 175000
  await request($, 'agent-1')
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    const bar = barProps(await ui.drawn(), /88% · 175k\/200k · 2 req · peak 180k$/)
    expect(bar?.color).toBe('#E95678')
    expect(bar?.dimColor).toBeFalsy()
    await ui.unmount()
  }
})

const withAutoCompact = BREAKDOWN

test('The pane draws the trend and the turns left', async ($, on) => {
  const r = rig(on)
  r.breakdown = withAutoCompact
  await start($)
  for (const t of [40000, 50000, 60000, 70000]) await measure($, r, t, t / 2000)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: /Trend/ })).toBeDefined()
    expect(await ui.find({ text: /▂▃▃▃\s*\+10k/ })).toBeDefined()
    expect(await ui.find({ text: /~10 turns to auto-compact at this pace/ })).toBeDefined()
    await ui.unmount()
  }
})

test('A shrunk window restarts the pace', async ($, on) => {
  const r = rig(on)
  r.breakdown = withAutoCompact
  await start($)
  for (const t of [100000, 150000, 60000, 70000]) await measure($, r, t, t / 2000)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: /~10 turns to auto-compact at this pace/ })).toBeDefined()
    await ui.unmount()
  }
})

test('A flat window is not growing', async ($, on) => {
  const r = rig(on)
  r.breakdown = withAutoCompact
  await start($)
  for (const t of [60000, 60000]) await measure($, r, t, t / 2000)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: /Trend/ })).toBeDefined()
    expect(await ui.find({ text: /not growing/ })).toBeDefined()
    await ui.unmount()
  }
})

test('The pane names the heaviest tool results', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.messages = withUses(
    toolUse('Read', { file_path: '/repo/crates/ratchet/src/usage/transcript.rs' }, 40000, 1),
    toolUse('Bash', { command: 'cargo test --workspace' }, 8000, 2),
    toolUse('Grep', { pattern: 'fn parse' }, 400, 3),
  )
  await start($)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    const text = textOf(await ui.drawn())
    expect(text).toMatch(
      /Top consumers[\s\S]*Read[\s\S]*10k[\s\S]*src\/usage\/transcript\.rs[\s\S]*Bash[\s\S]*2k[\s\S]*cargo test --workspace[\s\S]*Grep[\s\S]*100[\s\S]*fn parse/,
    )
    expect(await ui.find({ text: /→ reads this size can go to ratchet:reader/ })).toBeDefined()
    await ui.unmount()
  }
})

test('Only the five heaviest tool results are listed', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.messages = withUses(
    ...[600, 500, 400, 300, 200, 100].map((chars, i) => toolUse('Grep', { pattern: `p${i + 1}` }, chars, i)),
  )
  await start($)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    const text = textOf(await ui.drawn())
    for (const p of ['p1', 'p2', 'p3', 'p4', 'p5']) expect(text).toMatch(new RegExp(`\\b${p}\\b`))
    expect(text).not.toMatch(/\bp6\b/)
    await ui.unmount()
  }
})

test('A read under 5k tokens brings no reader hint', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.messages = withUses(toolUse('Read', { file_path: '/work/repo/README.md' }, 4000, 1))
  await start($)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    const text = textOf(await ui.drawn())
    expect(text).toMatch(/Read[\s\S]*1k[\s\S]*work\/repo\/README\.md/)
    expect(text).not.toMatch(/ratchet:reader/)
    await ui.unmount()
  }
})

const T99 = {
  tasks: [
    {
      id: 'T-0099',
      buckets: [
        tokensOf({ input: 10000, cache_write: 5000, cache_read: 40000, output: 4000, thinking: 1000 }, 'implementer'),
        tokensOf({ input: 1000 }, 'implementer'),
        tokensOf({ input: 20000, output: 10000, thinking: 3000 }, 'orchestrator'),
        tokensOf({ cache_read: 9000, output: 1000 }, 'ratchet:reviewer'),
      ],
    },
  ],
}

test('The pane shows what the held task has cost by role', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.tasks = [{ id: 'T-0001', title: 'done', status: 'done' }, inProgress('T-0099')]
  r.usageJson = T99
  await start($)
  await ctxCommand($)
  const bin = norm(`${ROOT}/bin/ratchet`)
  const runs = r.runs.map(argvNorm)
  expect(runs).toContainEqual([bin, 'task', 'list', '--mine', '--json', '--session', 'sess-1'])
  expect(runs).toContainEqual([bin, 'usage', 'T-0099', '--json'])
  for (const argv of runs) expect(argv[0]).toBe(bin)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: /T-0099\s*·\s*100k tokens so far/ })).toBeDefined()
    expect(await ui.find({ text: /implementer 60% · orchestrator 30% · reviewer 10%/ })).toBeDefined()
    await ui.unmount()
  }
})

test('On Windows the binary is ratchet.exe', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.tasks = [inProgress('T-0099')]
  r.usageJson = T99
  r.windows = true
  await start($)
  await ctxCommand($)
  const exe = norm(`${ROOT}/bin/ratchet.exe`)
  expect(r.runs.length).toBeGreaterThan(0)
  for (const argv of r.runs) expect(norm(argv[0])).toBe(exe)
})

test('Without a held task the pane shows no task cost', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.tasks = [
    { id: 'T-0001', title: 'queued', status: 'todo' },
    { id: 'T-0002', title: 'in review', status: 'review' },
  ]
  r.usageJson = T99
  await start($)
  await ctxCommand($)
  // Control: the module did ask the board.
  expect(r.runs.some(argv => argv.includes('list'))).toBe(true)
  expect(r.runs.some(argv => argv.includes('usage'))).toBe(false)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: /tokens so far/ })).toBeUndefined()
    await ui.unmount()
  }
})

const REPORT_PATH = '/home/u/.ratchet/out/20261004T143413-usage-T-0099.txt'
const longReport = () => {
  for (let n = 0; n < 100; n++) {
    const text = JSON.stringify({ ...T99, notes: Array.from({ length: n }, (_, i) => `n${i}`) }, null, 2)
    if (text.split('\n').length === 70) return text
  }
  throw new Error('no 70-line report')
}

test('A long report is read from the file its last line names', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.tasks = [inProgress('T-0099')]
  const report = longReport()
  r.reads[REPORT_PATH] = report
  r.usageStdout = `${report.split('\n').slice(0, 20).join('\n')}\n… (70 lines in ${REPORT_PATH})\n`
  await start($)
  await ctxCommand($)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: /T-0099\s*·\s*100k tokens so far/ })).toBeDefined()
    expect(await ui.find({ text: /implementer 60% · orchestrator 30% · reviewer 10%/ })).toBeDefined()
    await ui.unmount()
  }
})

test('A failed refresh keeps the last figures', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.tasks = [inProgress('T-0099')]
  r.usageJson = { tasks: [{ id: 'T-0099', buckets: [tokensOf({ input: 100000 }, 'implementer')] }] }
  await start($)
  await ctxCommand($)
  const first = await mountPane($, 'terminal')
  expect(await first.find({ text: /T-0099\s*·\s*100k tokens so far/ })).toBeDefined()
  await first.unmount()
  r.usageExit = 1
  await ctxCommand($)
  expect(r.runs.filter(argv => argv.includes('usage')).length).toBeGreaterThan(1)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: /T-0099\s*·\s*100k tokens so far/ })).toBeDefined()
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

test('A failing agent list leaves the line without its tail', async ($, on) => {
  const r = rig(on)
  r.agents = [AGENT1]
  await start($)
  // Control: with a working list a running subagent's request does tail the line.
  await measure($, r, 84000, 42)
  await request($, 'agent-1')
  expect(last(r)).toContain('│')
  r.agentsFail = true
  await measure($, r, 84000, 42)
  expect(last(r)).toBe(MAIN)
})

test('A failing command registration still meters the session', async ($, on) => {
  const r = rig(on)
  r.registerFails = true
  r.usage = { tokens: 84000, percent: 42, window: WINDOW }
  await start($)
  expect(last(r)).toBe(MAIN)
})

test('A failing read still draws the pane', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.usageFails = true
  await start($)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: /No breakdown yet/ })).toBeDefined()
    await ui.unmount()
  }
})

test("A failing agent list still records a subagent's request", async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.agents = [AGENT1]
  r.agentsFail = true
  await start($)
  await request($, 'agent-1')
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: /○ agent/ })).toBeDefined()
    const bar = barProps(await ui.drawn(), /25% · 50k\/200k · 1 req$/)
    expect(bar?.dimColor).toBe(true)
    await ui.unmount()
  }
})

test('A failing messages read draws the pane without top consumers', async ($, on) => {
  const r = rig(on)
  r.breakdown = BREAKDOWN
  r.messages = withUses(toolUse('Read', { file_path: '/work/repo/a.rs' }, 40000, 1))
  r.messagesFail = true
  await start($)
  for (const surface of SURFACES) {
    const ui = await mountPane($, surface)
    expect(await ui.find({ text: /43% · 86k \/ 200k/ })).toBeDefined()
    expect(await ui.find({ text: /Top consumers/ })).toBeUndefined()
    await ui.unmount()
  }
})
