# LexWisp Script Plugin API (Host API 1.x)

Script plugins are prepacked JavaScript ES modules. LexWisp loads only the package entry and relative `.js` imports inside that package. It provides no Node.js, npm, Deno, browser DOM, remote import, shell, arbitrary filesystem, raw credential, SQLite, HTTP client, or GPUI object.

The manifest uses `schema_version = 1`, `[plugin].kind = "script"`, `[plugin].entry`, and an exported `handler` for each text action. Parameters, allowed sources, dismiss policy, and output policy use the same fields as declarative actions. `network.request` also requires exact `[[capabilities.network]]` rules containing scheme, host, port, methods, and optional path prefixes.

```javascript
export async function run(ctx, input, params) {
  return { type: "text", text: input.text };
}
```

The immutable input snapshot contains `text` and `source`. Parameters are validated by Host before invocation. A handler either returns a string or `{ type: "text", text }`, or streams ordered deltas with `ctx.output.append(text)` and returns `{ type: "complete" }`; it cannot do both.

Available operations:

- `ctx.ai.invoke(input, system?)` returns collected model text and requires `ai.invoke`. Provider credentials never enter JavaScript.
- `ctx.http.request(method, url, body?)` returns a JSON string `{status, body}` and requires `network.request` plus a matching approved rule. Redirects, authentication/cookie headers, local/private addresses, non-UTF-8 bodies, and bodies over 2 MiB are rejected.
- `ctx.storage.get(key)`, `set(key, value)`, and `delete(key)` access only this plugin's 5 MiB namespace and require the corresponding storage capability.
- `ctx.context.snapshot` is the serialized snapshot authorized for this invocation; it contains no OS handles.
- `ctx.output.append(text)` appends bounded text to the same supervised execution.
- `ctx.ui.showResult()` asks Host to reveal the current result surface.
- `ctx.signal.isAborted()` reports cancellation.
- `ctx.log.write(level, category)` records identifiers/category only; do not pass user text or secrets.

One shared QuickJS VM has a 64 MiB limit and separate Context/module state per plugin. A synchronous slice is limited to 50 ms, cumulative JavaScript work to 2 seconds, total invocation wall time to 180 seconds, and output to 2 MiB UTF-8. Host I/O runs on LexWisp's business runtime and resolves bounded worker-queue Promises. Every callback retains the actual plugin, package hash, generation, and Invocation identity; disabling or reloading revokes old callbacks.

See `docs/lexwisp-plugin.d.ts` and the `script-text` / `script-multistep` examples. The multi-step endpoint is intentionally a placeholder and contains no credential.
