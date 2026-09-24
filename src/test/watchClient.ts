export function mockWatchClient(
	invoke: (command: string, args: Record<string, unknown>) => Promise<unknown>,
) {
	return (
		args: Record<string, unknown>,
		onReady: (id: number) => void,
		onError = console.error,
	) => {
		let disposed = false;
		let watcherId: number | undefined;
		const stop = (id: number) => {
			void invoke("stop_watching", { watcherId: id }).catch(onError);
		};
		void invoke("start_watching", args)
			.then((result) => {
				watcherId = Number(result);
				if (disposed) stop(watcherId);
				else onReady(watcherId);
			})
			.catch(onError);
		return () => {
			disposed = true;
			if (watcherId !== undefined) stop(watcherId);
		};
	};
}
