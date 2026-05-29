import { describe, expect, it } from 'vitest';
import {
  AGENT_INSPECTOR_PANEL,
  VEHICLE_INSPECTOR_PANEL,
  inspectorPanelLayout,
} from '../../src/render/inspectorPanelPainter';

describe('inspectorPanelPainter', () => {
  it('computes stable layout metrics for an inspector panel', () => {
    const layout = inspectorPanelLayout({
      title: 'Agent agent:1',
      rows: [
        { label: 'Tile', value: '10,20' },
        { label: 'Mode', value: 'walk' },
      ],
    }, AGENT_INSPECTOR_PANEL);

    expect(layout).toEqual({
      x: 12,
      y: 12,
      width: 232,
      height: 74,
      radius: 6,
      padding: 10,
      title: { x: 22, y: 22 },
      rows: [
        { label: 'Tile', value: '10,20', labelX: 22, valueX: 82, y: 42 },
        { label: 'Mode', value: 'walk', labelX: 22, valueX: 82, y: 59 },
      ],
    });
  });

  it('keeps agent and vehicle panel themes distinct', () => {
    expect(AGENT_INSPECTOR_PANEL).toEqual({
      x: 12,
      y: 12,
      accent: '#f7d76a',
      stroke: 'rgba(247, 215, 106, 0.8)',
    });
    expect(VEHICLE_INSPECTOR_PANEL).toEqual({
      x: 12,
      y: 128,
      accent: '#75d7ff',
      stroke: 'rgba(117, 215, 255, 0.8)',
    });
  });
});
