import DOMPurify from "dompurify";
import { Copy, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";

interface QrCodeData {
	url: string;
	svg: string;
	token_svg: string;
}

interface ServerConfig {
	port: number;
	token: string;
}

interface QrDisplayProps {
	qrData: QrCodeData;
	config: ServerConfig | null;
	onCopyUrl: () => void;
	onCopyToken: () => void;
	onRegenerateToken: () => void;
}

export function QrDisplay({
	qrData,
	config,
	onCopyUrl,
	onCopyToken,
	onRegenerateToken,
}: QrDisplayProps) {
	return (
		<>
			{/* Connection QR */}
			<div className="flex flex-col gap-2 border-t border-border pt-3">
				<span className="text-xs font-medium text-muted-foreground">
					Connection
				</span>
				<div
					className="w-full flex justify-center"
					// biome-ignore lint/security/noDangerouslySetInnerHtml: SVG sanitized by DOMPurify
					dangerouslySetInnerHTML={{
						__html: DOMPurify.sanitize(qrData.svg, {
							USE_PROFILES: { svg: true },
						}),
					}}
				/>
				<div className="flex items-center gap-1">
					<span className="flex-1 text-[10px] text-muted-foreground font-mono truncate">
						{qrData.url}
					</span>
					<Button
						variant="ghost"
						size="icon"
						className="size-5 shrink-0"
						onClick={onCopyUrl}
					>
						<Copy className="size-3" />
					</Button>
				</div>
			</div>

			{/* Auth Token QR */}
			{config && (
				<div className="flex flex-col gap-2 border-t border-border pt-3">
					<span className="text-xs font-medium text-muted-foreground">
						Auth Token
					</span>
					<div
						className="w-full flex justify-center"
						// biome-ignore lint/security/noDangerouslySetInnerHtml: SVG sanitized by DOMPurify
						dangerouslySetInnerHTML={{
							__html: DOMPurify.sanitize(qrData.token_svg, {
								USE_PROFILES: { svg: true },
							}),
						}}
					/>
					<div className="flex items-center gap-1">
						<span className="flex-1 text-[10px] text-muted-foreground font-mono truncate bg-muted border border-border rounded px-2 py-1">
							{config.token.slice(0, 8)}...
						</span>
						<Button
							variant="ghost"
							size="icon"
							className="size-5 shrink-0"
							onClick={onCopyToken}
							title="Copy token"
						>
							<Copy className="size-3" />
						</Button>
						<Button
							variant="ghost"
							size="icon"
							className="size-5 shrink-0"
							onClick={onRegenerateToken}
							title="Regenerate token"
						>
							<RefreshCw className="size-3" />
						</Button>
					</div>
				</div>
			)}
		</>
	);
}
