import { fileURLToPath } from "node:url";
import ts from "typescript";

const entry = fileURLToPath(new URL("../packages/native-modules.fixture.ts", import.meta.url));
const source = `
  import { Clipboard, getString } from "./clipboard/src/native.js";
  import { delete as remove, read } from "./file-system/src/native.js";
  import { Http, send as httpSend } from "./http/src/native.js";
  import { Linking, addEventListener } from "./linking/src/native.js";
  import { WebSocket as NativeWs, events as wsEvents, open as wsOpen } from "./websocket/src/native.js";

  const actual: string = Clipboard.getString();
  Clipboard.setString(getString());
  const pending: Promise<void> = remove("profile.json");
  const contents: Promise<Uint8Array | ArrayBuffer> = read("profile.json");
  const listener = ({ url }: { url: string }) => { void url; };
  addEventListener("url", listener);
  Linking.removeEventListener("url", listener);

  const httpStream = Http.send({ method: "GET", url: "https://example.test", redirect: "follow" });
  const httpDirect: typeof httpStream = httpSend({ method: "POST", url: "https://example.test", body: "hi" });
  void httpDirect;

  const wsHandle = NativeWs.open("wss://example.test/ws", ["chat"]);
  const wsId: number = wsHandle.call("id");
  const wsBuffered: number = wsHandle.call("sendText", "hello");
  const wsStream = wsEvents(wsId);
  wsHandle.call("close", 1000, "bye");
  wsHandle.close();
  void wsOpen;
  void wsBuffered;
  void wsStream;

  // @ts-expect-error File removal takes a path string.
  remove(42);
  // @ts-expect-error A host promise retains its resolved type.
  const sync: Uint8Array = read("profile.json");
  // @ts-expect-error Linking declares only the url event.
  addEventListener("change", listener);
  // @ts-expect-error Linking's payload carries a URL string.
  Linking.addEventListener("url", (event: { url: number }) => { void event; });
  // @ts-expect-error Http.send requires method and url.
  Http.send({ method: "GET" });
  // @ts-expect-error Unsupported redirect mode is rejected by NativeHttpRequest.
  Http.send({ method: "GET", url: "https://example.test", redirect: "always" });
  // @ts-expect-error WebSocket handle.call("sendBinary") requires bytes, not boolean.
  wsHandle.call("sendBinary", true);
  // @ts-expect-error WebSocket.events requires a numeric id.
  wsEvents("1");
`;
const options = {
  strict: true,
  noEmit: true,
  target: ts.ScriptTarget.ES2022,
  module: ts.ModuleKind.NodeNext,
};
const host = ts.createCompilerHost(options);
const getSourceFile = host.getSourceFile.bind(host);
host.getSourceFile = (path, languageOptions, ...args) =>
  path === entry
    ? ts.createSourceFile(path, source, languageOptions)
    : getSourceFile(path, languageOptions, ...args);

const program = ts.createProgram([entry], options, host);
const diagnostics = ts.getPreEmitDiagnostics(program);
if (diagnostics.length > 0) {
  process.stderr.write(ts.formatDiagnostics(diagnostics, host));
  process.exitCode = 1;
}
