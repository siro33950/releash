import { trackEvent as aptabaseTrackEvent } from "@aptabase/tauri";
import { invoke } from "@tauri-apps/api/core";

let _enabled = true;

export function setTelemetryEnabled(enabled: boolean): void {
	_enabled = enabled;
	invoke("update_telemetry_enabled", { enabled }).catch(() => {});
}

export function trackEvent(
	name: string,
	props?: Record<string, string | number>,
): void {
	if (!_enabled) return;
	aptabaseTrackEvent(name, props).catch(() => {});
}
