import type { AgentLod } from './agentLod';

export const AGENT_PALETTE = [0x3f6f8f, 0x7a6f48, 0x6f8a54, 0x8b4f4c, 0x4f6b59, 0x78608d, 0x9b7a3d, 0x5f6f7a] as const;

export function normalizeColorIndex(index: number): number {
  return ((index % AGENT_PALETTE.length) + AGENT_PALETTE.length) % AGENT_PALETTE.length;
}

export function agentColor(index: number): string {
  return `#${AGENT_PALETTE[normalizeColorIndex(index)].toString(16).padStart(6, '0')}`;
}

export function agentSize(lod: AgentLod): { width: number; height: number } {
  if (lod === 'citizen') return { width: 5, height: 8 };
  if (lod === 'pixel') return { width: 3, height: 4 };
  return { width: 2, height: 2 };
}
