import { projectIso } from '../projection';
import type { City, Tile } from '../types';
import { AgentRenderer } from './AgentRenderer';
import type { AgentRenderPlan } from './agentLod';

export class CityRenderer {
  readonly agentRenderer = new AgentRenderer();

  render(context: CanvasRenderingContext2D, city: City, agentPlan: AgentRenderPlan): void {
    context.clearRect(-10000, -10000, 20000, 20000);
    this.renderTerrain(context, city.tiles);
    this.renderRoads(context, city);
    this.agentRenderer.render(context, agentPlan);
    this.renderBuildings(context, city);
  }

  private renderTerrain(context: CanvasRenderingContext2D, tiles: Tile[]): void {
    for (const tile of tiles) {
      const iso = projectIso(tile.coord);
      context.fillStyle = terrainColor(tile.terrain);
      drawDiamond(context, iso.x, iso.y, 32, 16);
      context.fill();
      if (tile.terrain === 'riverbank') {
        context.strokeStyle = 'rgba(210, 220, 183, 0.18)';
        context.stroke();
      }
    }
  }

  private renderRoads(context: CanvasRenderingContext2D, city: City): void {
    context.save();
    context.lineCap = 'round';
    context.lineJoin = 'round';
    for (const road of city.roadEdges) {
      context.beginPath();
      road.points.forEach((point, index) => {
        const iso = projectIso(point);
        if (index === 0) context.moveTo(iso.x, iso.y - 2);
        else context.lineTo(iso.x, iso.y - 2);
      });
      context.strokeStyle = road.hierarchy === 'primary' ? '#5b5650' : road.hierarchy === 'secondary' ? '#67635c' : '#736d62';
      context.lineWidth = road.hierarchy === 'primary' ? 7 : road.hierarchy === 'secondary' ? 5 : 3;
      context.stroke();
      context.strokeStyle = 'rgba(190, 178, 148, 0.5)';
      context.lineWidth = 1;
      context.stroke();
    }
    context.restore();
  }

  private renderBuildings(context: CanvasRenderingContext2D, city: City): void {
    const buildings = [...city.buildings].sort((a, b) => a.coord.x + a.coord.y - (b.coord.x + b.coord.y));
    for (const building of buildings) {
      const iso = projectIso(building.coord);
      const width = 12 + building.width * 5;
      const height = 8 + building.height * 7;
      context.fillStyle = buildingColor(building.kind);
      context.fillRect(iso.x - width / 2, iso.y - height, width, height);
      context.fillStyle = 'rgba(255, 246, 190, 0.18)';
      context.fillRect(iso.x - width / 2 + 2, iso.y - height + 3, Math.max(3, width - 5), 2);
      context.strokeStyle = 'rgba(20, 20, 18, 0.36)';
      context.strokeRect(iso.x - width / 2, iso.y - height, width, height);
    }
  }
}

function drawDiamond(context: CanvasRenderingContext2D, x: number, y: number, width: number, height: number): void {
  context.beginPath();
  context.moveTo(x, y - height / 2);
  context.lineTo(x + width / 2, y);
  context.lineTo(x, y + height / 2);
  context.lineTo(x - width / 2, y);
  context.closePath();
}

function terrainColor(terrain: Tile['terrain']): string {
  if (terrain === 'water') return '#315f72';
  if (terrain === 'riverbank') return '#7b8a62';
  if (terrain === 'park') return '#52794f';
  if (terrain === 'plaza') return '#8b8060';
  return '#657d52';
}

function buildingColor(kind: string): string {
  if (kind === 'commercial') return '#8d6f58';
  if (kind === 'civic') return '#8a8580';
  if (kind === 'industrial') return '#6f6a62';
  if (kind === 'park') return '#4a6f4c';
  return '#7c6752';
}
