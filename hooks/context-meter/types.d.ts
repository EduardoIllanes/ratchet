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

declare module 'claude-code' {
  interface PluginState {
    ratchet: {
      contextReading: Reading | null
      contextAlerted: number
      contextAgents: Record<string, AgentReading>
      contextMetered: boolean
    }
  }
}
