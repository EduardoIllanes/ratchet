export type Reading = {
  tokens: number | null
  window: number
  percent: number | null
}

/** One subagent's context, as its last model request reported it. */
export type AgentReading = {
  tokens: number
  peak: number
  steps: number
  model: string
  at: number
  /** Captured while the agent was still in `$.agent.list()`, which drops it once done. */
  type: string
  description: string
}

/** What the task this session holds has cost so far, from `ratchet usage <id> --json`. */
export type TaskCost = {
  id: string
  title: string
  total: number
  roles: { role: string; tokens: number }[]
}

declare module 'claude-code' {
  interface PluginState {
    ratchet: {
      contextReading: Reading | null
      contextAlerted: number
      contextAgents: Record<string, AgentReading>
      contextHistory: number[]
      contextTask: TaskCost | null
      contextMetered: boolean
    }
  }
}
