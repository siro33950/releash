import { useEffect, useState } from "react";
import { QrTokenScanner } from "./QrTokenScanner";

const STORAGE_KEY = "releash-connection";

interface ConnectionInfo {
	host: string;
	token: string;
}

function loadSavedConnection(): ConnectionInfo {
	try {
		const saved = localStorage.getItem(STORAGE_KEY);
		if (saved) return JSON.parse(saved) as ConnectionInfo;
	} catch {
		// ignore
	}
	return { host: "", token: "" };
}

function saveConnection(info: ConnectionInfo) {
	localStorage.setItem(STORAGE_KEY, JSON.stringify(info));
}

interface ConnectionFormProps {
	onConnect: (wsUrl: string, token: string) => void;
}

export function ConnectionForm({ onConnect }: ConnectionFormProps) {
	const [host, setHost] = useState("");
	const [token, setToken] = useState("");
	const [scanning, setScanning] = useState(false);

	useEffect(() => {
		const saved = loadSavedConnection();
		setHost(saved.host || window.location.host);
		setToken(saved.token || "");
	}, []);

	const handleSubmit = (e: React.FormEvent) => {
		e.preventDefault();
		if (!host || !token) return;
		const wsUrl = `${window.location.protocol === "https:" ? "wss" : "ws"}://${host}/ws`;
		saveConnection({ host, token });
		onConnect(wsUrl, token);
	};

	const handleScan = (scannedToken: string) => {
		setToken(scannedToken);
		setScanning(false);
	};

	return (
		<div className="flex items-center justify-center min-h-dvh bg-neutral-950 text-neutral-100">
			<form
				onSubmit={handleSubmit}
				className="w-full max-w-md p-4 sm:p-8 space-y-6 bg-neutral-900 rounded-xl border border-neutral-800"
			>
				<h1 className="text-2xl font-bold text-center">Releash Remote</h1>
				<p className="text-sm text-neutral-400 text-center">
					Mac上のReleashに接続します
				</p>

				<div className="space-y-2">
					<label
						className="block text-sm font-medium text-neutral-300"
						htmlFor="host"
					>
						ホスト (IP:ポート)
					</label>
					<input
						id="host"
						type="text"
						value={host}
						onChange={(e) => setHost(e.target.value)}
						placeholder="192.168.1.100:9700"
						className="w-full px-4 py-2 rounded-lg bg-neutral-800 border border-neutral-700 text-neutral-100 placeholder-neutral-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
					/>
				</div>

				<div className="space-y-2">
					<label
						className="block text-sm font-medium text-neutral-300"
						htmlFor="token"
					>
						トークン
					</label>
					<div className="flex gap-2">
						<input
							id="token"
							type="password"
							value={token}
							onChange={(e) => setToken(e.target.value)}
							placeholder="認証トークンを入力"
							className="flex-1 px-4 py-2 rounded-lg bg-neutral-800 border border-neutral-700 text-neutral-100 placeholder-neutral-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
						/>
						<button
							type="button"
							onClick={() => setScanning(true)}
							className="px-3 py-2 rounded-lg bg-neutral-700 hover:bg-neutral-600 text-sm font-medium transition-colors shrink-0"
						>
							QRスキャン
						</button>
					</div>
				</div>

				<button
					type="submit"
					disabled={!host || !token}
					className="w-full py-3 rounded-lg bg-blue-600 hover:bg-blue-500 disabled:opacity-40 disabled:cursor-not-allowed font-medium transition-colors"
				>
					接続
				</button>
			</form>

			{scanning && (
				<QrTokenScanner
					onScan={handleScan}
					onClose={() => setScanning(false)}
				/>
			)}
		</div>
	);
}
