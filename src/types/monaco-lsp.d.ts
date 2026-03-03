/**
 * Module augmentation for monaco.lsp namespace (available since Monaco 0.55.0).
 *
 * These APIs are not yet part of the official TypeScript definitions.
 * When Monaco officially exports these types, this file can be removed.
 *
 * @see https://github.com/microsoft/monaco-editor/blob/main/CHANGELOG.md
 */
export {};

declare module "monaco-editor" {
	namespace lsp {
		interface IMessageTransport {
			readonly state: {
				readonly value:
					| { state: "connecting" }
					| { state: "open" }
					| { state: "closed"; error: Error | undefined };
				readonly onChange: (
					listener: (
						e:
							| { state: "connecting" }
							| { state: "open" }
							| { state: "closed"; error: Error | undefined },
					) => void,
				) => { dispose(): void };
			};
			send(message: unknown): Promise<void>;
			setListener(listener: ((message: unknown) => void) | undefined): void;
			toString(): string;
		}

		class MonacoLspClient {
			constructor(transport: IMessageTransport);
			dispose(): void;
		}
	}
}
