type ScrollMetrics = {
	scrollHeight: number;
	scrollTop: number;
	clientHeight: number;
};

type ScrollMetricsPatch = Partial<ScrollMetrics>;

export function installScrollMetricsMock(
	target: HTMLElement,
	{ scrollHeight, scrollTop, clientHeight }: ScrollMetrics,
) {
	let currentScrollHeight = scrollHeight;
	let currentScrollTop = scrollTop;
	let currentClientHeight = clientHeight;
	const originalScrollHeight = Object.getOwnPropertyDescriptor(
		target,
		"scrollHeight",
	);
	const originalScrollTop = Object.getOwnPropertyDescriptor(
		target,
		"scrollTop",
	);
	const originalClientHeight = Object.getOwnPropertyDescriptor(
		target,
		"clientHeight",
	);

	Object.defineProperty(target, "scrollHeight", {
		configurable: true,
		get: () => currentScrollHeight,
	});
	Object.defineProperty(target, "scrollTop", {
		configurable: true,
		get: () => currentScrollTop,
		set: (value: number) => {
			currentScrollTop = value;
		},
	});
	Object.defineProperty(target, "clientHeight", {
		configurable: true,
		get: () => currentClientHeight,
	});

	return {
		get scrollTop() {
			return currentScrollTop;
		},
		set scrollTop(value: number) {
			currentScrollTop = value;
		},
		setMetrics(metrics: ScrollMetricsPatch) {
			currentScrollHeight = metrics.scrollHeight ?? currentScrollHeight;
			currentScrollTop = metrics.scrollTop ?? currentScrollTop;
			currentClientHeight = metrics.clientHeight ?? currentClientHeight;
		},
		restore() {
			if (originalScrollHeight) {
				Object.defineProperty(target, "scrollHeight", originalScrollHeight);
			} else {
				Reflect.deleteProperty(target, "scrollHeight");
			}
			if (originalScrollTop) {
				Object.defineProperty(target, "scrollTop", originalScrollTop);
			} else {
				Reflect.deleteProperty(target, "scrollTop");
			}
			if (originalClientHeight) {
				Object.defineProperty(target, "clientHeight", originalClientHeight);
			} else {
				Reflect.deleteProperty(target, "clientHeight");
			}
		},
	};
}
