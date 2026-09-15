import { useSyncExternalStore } from "react";
import { Button } from "@/components/ui/button";
import {
	dismissClientOperation,
	getClientStatus,
	retryClientOperation,
	subscribeClientStatus,
} from "@/lib/clientSocket";

export function ClientConnectionBanner() {
	const status = useSyncExternalStore(subscribeClientStatus, getClientStatus);
	if (!status.message && status.operations.length === 0) return null;
	return (
		<div
			role="status"
			className="border-b border-border bg-muted px-4 py-2 text-sm"
		>
			{status.message && <p>{status.message}</p>}
			{status.operations.map((operation) => (
				<div key={operation.id} className="flex items-center gap-2">
					{operation.state === "not_sent" ? (
						<>
							<span>
								{operation.command}:{" "}
								{operation.expired
									? "要求は送信されていません（未実行）。"
									: "接続の回復を待っています（未送信）。"}
							</span>
							{operation.expired && (
								<Button
									variant="outline"
									size="sm"
									onClick={() => dismissClientOperation(operation.id)}
								>
									確認済み
								</Button>
							)}
						</>
					) : (
						<>
							<span>{operation.command}: 操作結果を確認できません。</span>
							<Button
								variant="outline"
								size="sm"
								disabled={!status.connected}
								onClick={() => retryClientOperation(operation.id)}
							>
								元の操作の結果を確認
							</Button>
						</>
					)}
				</div>
			))}
		</div>
	);
}
