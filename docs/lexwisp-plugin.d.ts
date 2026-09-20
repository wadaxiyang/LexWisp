export type InputSource = "manual" | "selection" | "candidate" | "clipboard";

export interface ActionInput {
  readonly text: string;
  readonly source: InputSource;
}

export interface ScriptContext {
  readonly ai: {
    invoke(input: string, system?: string): Promise<string>;
  };
  readonly http: {
    request(method: string, url: string, body?: string): Promise<string>;
  };
  readonly storage: {
    get(key: string): Promise<string>;
    set(key: string, value: string): Promise<string>;
    delete(key: string): Promise<string>;
  };
  readonly context: { readonly snapshot: Readonly<Record<string, unknown>> };
  readonly output: { append(text: string): void };
  readonly ui: { showResult(): void };
  readonly signal: { isAborted(): boolean };
  readonly log: { write(level: "debug" | "info" | "warn" | "error", category: string): void };
}

export type ScriptResult = string | { type: "text"; text: string } | { type: "complete" };
export type ScriptHandler = (
  ctx: ScriptContext,
  input: Readonly<ActionInput>,
  params: Readonly<Record<string, string>>,
) => ScriptResult | Promise<ScriptResult>;
