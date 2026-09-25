import { useState } from "react";
import { useStateSubscription } from "@/hooks/useStateSubscription";

export function BackgroundFailures({ target }: { target?: string | null }) {
	const [open, setOpen] = useState(false);
	const [page, setPage] = useState({ target, offset: 0 });
	const offset = page.target === target ? page.offset : 0;
	const records = useStateSubscription(
		target
			? { kind: "failures", args: offset ? [target, String(offset)] : [target] }
			: null,
	);
	const items = records?.items ?? [];
	if (!records || (!items.length && !offset)) return null;
	return (
		<details
			open={open}
			onToggle={(event) => setOpen(event.currentTarget.open)}
			className="relative shrink-0 rounded bg-background p-1 text-xs"
		>
			<summary className="cursor-pointer">
				{records.requiresAttention ? "要対応" : "処理の失敗"}
			</summary>
			<ul className="absolute right-0 top-full z-50 max-h-64 w-72 space-y-2 overflow-auto rounded border bg-background p-2 shadow-md">
				{items.map((record) => (
					<li
						key={`${record.target}:${record.operation}:${record.classification}`}
					>
						<div>{record.target}</div>
						<div>
							{record.operation} · {record.classification}
						</div>
						<span>{record.message}</span>
						<div>{record.count}回</div>
						<div>初回: {new Date(record.firstObservedMs).toLocaleString()}</div>
						<div>最終: {new Date(record.lastObservedMs).toLocaleString()}</div>
					</li>
				))}
				<li className="flex justify-between">
					<button
						type="button"
						disabled={offset === 0}
						onClick={() =>
							setPage({ target, offset: Math.max(0, offset - 100) })
						}
					>
						前へ
					</button>
					<button
						type="button"
						disabled={records.nextOffset == null}
						onClick={() => setPage({ target, offset: records.nextOffset ?? 0 })}
					>
						次へ
					</button>
				</li>
			</ul>
		</details>
	);
}
