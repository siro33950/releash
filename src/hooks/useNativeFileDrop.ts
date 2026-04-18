import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef } from "react";

export interface NativeFileDropPayload {
	paths: string[];
	position: [number, number];
}

interface UseNativeFileDropOptions {
	onDropToEditor: (paths: string[]) => void;
}

export type DropZoneType = "editor" | "agent";

export function resolveZone(
	zones: Map<DropZoneType, HTMLElement>,
	target: EventTarget | null,
): DropZoneType | null {
	if (!(target instanceof Node)) return null;
	for (const [zone, element] of zones) {
		if (element.contains(target)) {
			return zone;
		}
	}
	return null;
}

export function hasVisibleZones(
	zones: Map<DropZoneType, HTMLElement>,
): boolean {
	for (const element of zones.values()) {
		const rect = element.getBoundingClientRect();
		if (rect.width > 0 && rect.height > 0) return true;
	}
	return false;
}

export function useNativeFileDrop(options: UseNativeFileDropOptions) {
	const zonesRef = useRef<Map<DropZoneType, HTMLElement>>(new Map());
	const callbacksRef = useRef<Map<DropZoneType, (paths: string[]) => void>>(
		new Map(),
	);
	const optionsRef = useRef(options);
	optionsRef.current = options;

	const lastZoneRef = useRef<DropZoneType | null>(null);

	// dragoverイベントでカーソル直下のゾーンを追跡
	useEffect(() => {
		const handleDragOver = (e: DragEvent) => {
			lastZoneRef.current = resolveZone(zonesRef.current, e.target);
		};
		document.addEventListener("dragover", handleDragOver);
		return () => document.removeEventListener("dragover", handleDragOver);
	}, []);

	useEffect(() => {
		const unlisten = listen<NativeFileDropPayload>(
			"native-file-drop",
			(event) => {
				const { paths } = event.payload;
				if (paths.length === 0) return;

				// display:none の画面はゾーンが0サイズになるためスキップ
				if (!hasVisibleZones(zonesRef.current)) return;

				const zone = lastZoneRef.current;
				const callback = zone ? callbacksRef.current.get(zone) : undefined;

				if (callback) {
					callback(paths);
				} else if (zone === "editor") {
					optionsRef.current.onDropToEditor(paths);
				}
				// zone === null の場合は何もしない（TerminalPanelが自己処理する）

				lastZoneRef.current = null;
			},
		);

		return () => {
			unlisten.then((f) => f());
		};
	}, []);

	const registerDropZone = useCallback(
		(
			zone: DropZoneType,
			element: HTMLElement | null,
			onDrop?: (paths: string[]) => void,
		) => {
			if (element) {
				zonesRef.current.set(zone, element);
				if (onDrop) {
					callbacksRef.current.set(zone, onDrop);
				} else {
					callbacksRef.current.delete(zone);
				}
			} else {
				zonesRef.current.delete(zone);
				callbacksRef.current.delete(zone);
			}
		},
		[],
	);

	return { registerDropZone };
}
