import { Html5Qrcode } from "html5-qrcode";
import { useEffect, useRef } from "react";

interface QrTokenScannerProps {
	onScan: (token: string) => void;
	onClose: () => void;
}

const READER_ID = "qr-token-reader";

export function QrTokenScanner({ onScan, onClose }: QrTokenScannerProps) {
	const onScanRef = useRef(onScan);
	onScanRef.current = onScan;

	useEffect(() => {
		let stopped = false;
		const scanner = new Html5Qrcode(READER_ID);

		scanner
			.start(
				{ facingMode: "environment" },
				{ fps: 10, qrbox: { width: 250, height: 250 } },
				(decodedText) => {
					if (stopped) return;
					stopped = true;
					scanner.stop().then(
						() => onScanRef.current(decodedText),
						() => onScanRef.current(decodedText),
					);
				},
				() => {},
			)
			.catch(() => {});

		return () => {
			if (!stopped) {
				stopped = true;
				scanner.stop().catch(() => {});
			}
		};
	}, []);

	return (
		<div className="fixed inset-0 z-50 flex items-center justify-center bg-black/80">
			<div className="w-full max-w-sm mx-4 bg-neutral-900 rounded-xl border border-neutral-700 overflow-hidden">
				<div className="flex items-center justify-between px-4 py-3 border-b border-neutral-700">
					<span className="text-sm font-medium text-neutral-100">
						トークンQRをスキャン
					</span>
					<button
						type="button"
						onClick={onClose}
						className="text-neutral-400 hover:text-neutral-100 text-lg leading-none"
					>
						&times;
					</button>
				</div>
				<div id={READER_ID} className="w-full" />
			</div>
		</div>
	);
}
