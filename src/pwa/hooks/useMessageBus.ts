import { useCallback, useRef } from "react";
import type { WsMessage } from "@/types/protocol";

type MessageHandler = (msg: WsMessage) => void;

export type Subscribe = (handler: MessageHandler) => () => void;

export function useMessageBus() {
	const subscribersRef = useRef(new Set<MessageHandler>());

	const dispatch = useCallback((msg: WsMessage) => {
		for (const sub of subscribersRef.current) {
			sub(msg);
		}
	}, []);

	const subscribe: Subscribe = useCallback((handler: MessageHandler) => {
		subscribersRef.current.add(handler);
		return () => {
			subscribersRef.current.delete(handler);
		};
	}, []);

	return { dispatch, subscribe };
}
