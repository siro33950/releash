interface ShimmerPlaceholderProps {
	lines?: number;
}

function shimmerLines(count: number): React.ReactNode[] {
	const result: React.ReactNode[] = [];
	for (let i = 0; i < count; i++) {
		result.push(
			<div
				key={`s${i}`}
				className={`h-3 agent-shimmer ${i === count - 1 ? "w-3/5" : "w-full"}`}
			/>,
		);
	}
	return result;
}

export function ShimmerPlaceholder({ lines = 3 }: ShimmerPlaceholderProps) {
	return (
		<div data-testid="shimmer-placeholder" className="px-5 py-3 space-y-2">
			{shimmerLines(lines)}
		</div>
	);
}
