import { useRef } from "react";
import { useTerminal } from "@/hooks/useTerminal";
import "@xterm/xterm/css/xterm.css";

export function TerminalPanel() {
	const containerRef = useRef<HTMLDivElement>(null);
	useTerminal(containerRef);

	return <div ref={containerRef} className="h-full w-full p-2 bg-[#1a1a1a]" />;
}
