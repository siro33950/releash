import { useCallback, useEffect, useRef, useState } from "react";
import type { WsMessage } from "@/types/protocol";
import type { Subscribe } from "./useMessageBus";

interface FileContent {
	path: string;
	original: string;
	modified: string;
}

interface UseRemoteFileContentOptions {
	subscribe: Subscribe;
	send: (msg: WsMessage) => void;
}

export function useRemoteFileContent({ subscribe, send }: UseRemoteFileContentOptions) {
	const [content, setContent] = useState<FileContent | null>(null);
	const [loading, setLoading] = useState(false);
	const currentPathRef = useRef<string | null>(null);

	const requestContent = useCallback(
		(path: string) => {
			currentPathRef.current = path;
			setLoading(true);
			send({ type: "file_content_request", payload: { path } });
		},
		[send],
	);

	useEffect(() => {
		return subscribe((msg: WsMessage) => {
			if (msg.type === "file_content_response") {
				if (msg.payload.path === currentPathRef.current) {
					setContent({
						path: msg.payload.path,
						original: msg.payload.original,
						modified: msg.payload.modified,
					});
					setLoading(false);
				}
			} else if (msg.type === "file_change") {
				if (msg.payload.path === currentPathRef.current) {
					send({
						type: "file_content_request",
						payload: { path: currentPathRef.current },
					});
				}
			}
		});
	}, [subscribe, send]);

	const clear = useCallback(() => {
		currentPathRef.current = null;
		setContent(null);
		setLoading(false);
	}, []);

	return { content, loading, requestContent, clear };
}
