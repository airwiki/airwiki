import { Channel, invoke } from '@tauri-apps/api/core';

export interface AppSnapshot {
  schemaVersion: number;
  sequence: number;
  phase: 'starting';
}

export interface UiEventEnvelope {
  schemaVersion: number;
  sequence: number;
  kind: string;
}

export async function connect(onEvent: (event: UiEventEnvelope) => void): Promise<AppSnapshot> {
  const events = new Channel<UiEventEnvelope>();
  events.onmessage = onEvent;
  return invoke<AppSnapshot>('connect', { events });
}

export async function installModels(): Promise<void> {
  return invoke('install_models');
}

export async function cancelModelInstall(): Promise<void> {
  return invoke('cancel_model_install');
}
