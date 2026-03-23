import { invoke } from "@tauri-apps/api/core";
import { useSyncExternalStore } from "react";

export interface SlashCommand {
	name: string;
	description: string;
	argumentHint?: string;
}

type Listener = () => void;

let cache: SlashCommand[] = [];
const listeners = new Set<Listener>();

function emitChange() {
	for (const listener of listeners) {
		listener();
	}
}

export function setSlashCommands(commands: SlashCommand[]): void {
	cache = commands;
	emitChange();
}

function subscribe(listener: Listener): () => void {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

function getSnapshot(): SlashCommand[] {
	return cache;
}

export async function loadSlashCommands(cwd: string): Promise<void> {
	const commands = await invoke<SlashCommand[]>("scan_slash_commands", {
		cwd,
	});
	setSlashCommands(commands);
}

export function useSlashCommands(): SlashCommand[] {
	return useSyncExternalStore(subscribe, getSnapshot);
}
