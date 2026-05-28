import {
  requireBackend as requireBackendGate,
  type BackendHealthDto,
} from '../backend/backendGate';
import {
  requireBaseWorld as requireBaseWorldClient,
  type BaseWorldResponse,
} from '../backend/baseWorldClient';
import {
  connectMobilityBackend as connectMobilityBackendClient,
  requireMobilitySnapshot as requireMobilitySnapshotClient,
  type MobilityBackendBridge,
  type MobilityBackendBridgeOptions,
  type MobilityViewportGetters,
  type RequiredMobility,
} from '../backend/mobilityClient';
import type { MobilityOverlayState } from '../backend/mobilityState';
import {
  mountCardHandView as mountCardHandViewRuntime,
  type CardHandViewOptions,
} from '../cardHand/cardHandView';

export type AppRuntimeInitialState = {
  backendStatus: BackendHealthDto;
  baseWorld: BaseWorldResponse;
  mobilityState: MobilityOverlayState;
  mobilityTickPeriodMs: number;
};

export type AppRuntimeHandle = {
  mobilityBackendBridge: MobilityBackendBridge | null;
  stop: () => void;
};

export type AppRuntimeDependencies = {
  requireBackend: (options: { baseUrl: string }) => Promise<BackendHealthDto>;
  requireBaseWorld: (options: { baseUrl: string }) => Promise<BaseWorldResponse>;
  requireMobilitySnapshot: (options: { baseUrl: string }) => Promise<RequiredMobility>;
  mountCardHandView: (options: CardHandViewOptions) => void;
  boot: (initialState: AppRuntimeInitialState) => void | Promise<void>;
  connectMobilityBackend: (options: MobilityBackendBridgeOptions) => MobilityBackendBridge;
  renderBackendRequired: (error: unknown) => void;
  addBeforeUnloadListener: (listener: () => void) => void;
};

export type StartAppRuntimeOptions = {
  backendBaseUrl: string;
  viewport: MobilityViewportGetters;
  onInitialState: (initialState: AppRuntimeInitialState) => void;
  onMobilityState: (state: MobilityOverlayState) => void;
  dependencies: AppRuntimeDependencies;
};

export function browserBeforeUnload(listener: () => void): void {
  window.addEventListener('beforeunload', listener, { once: true });
}

export function defaultAppRuntimeDependencies(
  boot: (initialState: AppRuntimeInitialState) => void | Promise<void>,
  renderBackendRequired: (error: unknown) => void,
): AppRuntimeDependencies {
  return {
    requireBackend: requireBackendGate,
    requireBaseWorld: requireBaseWorldClient,
    requireMobilitySnapshot: requireMobilitySnapshotClient,
    mountCardHandView: mountCardHandViewRuntime,
    boot,
    connectMobilityBackend: connectMobilityBackendClient,
    renderBackendRequired,
    addBeforeUnloadListener: browserBeforeUnload,
  };
}

export async function startAppRuntime(options: StartAppRuntimeOptions): Promise<AppRuntimeHandle> {
  const { backendBaseUrl, dependencies } = options;

  try {
    const backendStatus = await dependencies.requireBackend({ baseUrl: backendBaseUrl });
    const baseWorld = await dependencies.requireBaseWorld({ baseUrl: backendBaseUrl });
    const required = await dependencies.requireMobilitySnapshot({ baseUrl: backendBaseUrl });
    const initialState: AppRuntimeInitialState = {
      backendStatus,
      baseWorld,
      mobilityState: required.state,
      mobilityTickPeriodMs: required.tickPeriodMs,
    };

    options.onInitialState(initialState);
    dependencies.mountCardHandView({ baseUrl: backendBaseUrl });
    await dependencies.boot(initialState);
    const mobilityBackendBridge = dependencies.connectMobilityBackend({
      baseUrl: backendBaseUrl,
      initialState: required.state,
      onState: options.onMobilityState,
      viewport: options.viewport,
    });

    dependencies.addBeforeUnloadListener(() => mobilityBackendBridge.stop());

    return {
      mobilityBackendBridge,
      stop: () => mobilityBackendBridge.stop(),
    };
  } catch (error) {
    dependencies.renderBackendRequired(error);
    return {
      mobilityBackendBridge: null,
      stop: () => {},
    };
  }
}
