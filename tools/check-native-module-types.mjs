import { fileURLToPath } from "node:url";
import ts from "typescript";

const entry = fileURLToPath(new URL("../packages/native-modules.fixture.ts", import.meta.url));
const source = `
  import { Clipboard, getString } from "./clipboard/src/native.js";
  import { delete as remove, read } from "./file-system/src/native.js";
  import { Linking, addEventListener } from "./linking/src/native.js";

  const actual: string = Clipboard.getString();
  Clipboard.setString(getString());
  const pending: Promise<void> = remove("profile.json");
  const contents: Promise<Uint8Array | ArrayBuffer> = read("profile.json");
  const listener = ({ url }: { url: string }) => { void url; };
  addEventListener("url", listener);
  Linking.removeEventListener("url", listener);

  // @ts-expect-error File removal takes a path string.
  remove(42);
  // @ts-expect-error A host promise retains its resolved type.
  const sync: Uint8Array = read("profile.json");
  // @ts-expect-error Linking declares only the url event.
  addEventListener("change", listener);
  // @ts-expect-error Linking's payload carries a URL string.
  Linking.addEventListener("url", (event: { url: number }) => { void event; });
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
