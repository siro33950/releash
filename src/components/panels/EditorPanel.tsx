import { useRef } from "react";
import { useMonacoEditor } from "@/hooks/useMonacoEditor";

export function EditorPanel() {
	const containerRef = useRef<HTMLDivElement>(null);

	useMonacoEditor(containerRef, {
		defaultValue: "// Welcome to Releash Editor\n",
		language: "typescript",
	});

	return <div ref={containerRef} className="h-full w-full" />;
}
