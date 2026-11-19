import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const root = fileURLToPath(new URL("../", import.meta.url));
const fixture = readFileSync(new URL("../crates/js/tests/fixtures/standard.ts", import.meta.url), "utf8");
const options = {
  strict: true,
  noEmit: true,
  target: ts.ScriptTarget.ES2022,
  lib: ["lib.es2022.d.ts"],
  types: [],
};

function rust(profile, action, input) {
  return execFileSync("cargo", ["run", "--quiet", "--locked", "-p", "js", "--example", "standard-contract", "--", profile, action], {
    cwd: root,
    input,
    encoding: "utf8",
    timeout: 120_000,
  });
}

function compile(profile, declarations, source) {
  const declarationPath = resolve(root, "standard.d.ts");
  const fixturePath = resolve(root, "standard.ts");
  const virtualFiles = new Map([[declarationPath, declarations], [fixturePath, source]]);
  const host = ts.createCompilerHost(options);
  const readFile = host.readFile;
  host.readFile = path => virtualFiles.get(path) ?? readFile(path);
  const program = ts.createProgram([...virtualFiles.keys()], options, host);
  const diagnostics = ts.getPreEmitDiagnostics(program);
  assert.equal(diagnostics.length, 0, `${profile}\n${ts.formatDiagnosticsWithColorAndContext(diagnostics, host)}`);
  return program.getSourceFile(declarationPath);
}

function memberName(name) {
  if (ts.isIdentifier(name) || ts.isStringLiteral(name) || ts.isNumericLiteral(name)) return name.text;
  if (ts.isComputedPropertyName(name) && ts.isPropertyAccessExpression(name.expression)
      && ts.isIdentifier(name.expression.expression) && name.expression.expression.text === "Symbol") {
    return `[Symbol.${name.expression.name.text}]`;
  }
  throw new Error(`Unsupported declaration member ${name.getText()}`);
}

function members(nodes) {
  return Object.fromEntries(nodes.map(node => {
    const method = ts.isMethodDeclaration(node) || ts.isMethodSignature(node);
    return [memberName(node.name), method ? "method" : "property"];
  }));
}

function declaredSurface(file) {
  const surface = {};
  for (const node of file.statements) {
    if (ts.isClassDeclaration(node)) {
      const constructor = node.members.find(ts.isConstructorDeclaration);
      const parent = node.heritageClauses?.find(clause => clause.token === ts.SyntaxKind.ExtendsKeyword);
      const visible = node.members.filter(member => member.name && !ts.hasSyntacticModifier(member, ts.ModifierFlags.Private));
      const isStatic = member => ts.hasSyntacticModifier(member, ts.ModifierFlags.Static);
      surface[node.name.text] = {
        kind: "class",
        constructible: !constructor || !ts.hasSyntacticModifier(constructor, ts.ModifierFlags.Private),
        parent: parent?.types[0].expression.getText() ?? null,
        static: members(visible.filter(isStatic)),
        prototype: members(visible.filter(member => !isStatic(member))),
      };
    } else if (ts.isFunctionDeclaration(node)) {
      surface[node.name.text] = { kind: "function" };
    } else if (ts.isVariableStatement(node)) {
      for (const { name, type } of node.declarationList.declarations) {
        assert.ok(type && ts.isTypeLiteralNode(type), `Expected an object type for ${name.getText()}`);
        surface[memberName(name)] = { kind: "object", own: members(type.members) };
      }
    } else {
      assert.ok(ts.isInterfaceDeclaration(node) || ts.isTypeAliasDeclaration(node), node.getText());
    }
  }
  return surface;
}

// This function also runs in the Rust harness's QuickJS context.
function inspectSurface(baseline, declared) {
  function memberNames(object, implicit) {
    return Object.fromEntries(Reflect.ownKeys(object).filter(key => !implicit.includes(key)).map(key => {
      const descriptor = Object.getOwnPropertyDescriptor(object, key);
      const name = typeof key === "symbol" ? `[${key.description}]` : key;
      return [name, typeof descriptor.value === "function" ? "method" : "property"];
    }));
  }

  const surface = {};
  for (const key of Reflect.ownKeys(globalThis)) {
    // QuickJS already supplies some globals, such as performance, that the
    // runtime replaces. Inspect those when they belong to the public contract.
    if ((baseline.includes(key) && !Object.hasOwn(declared, key)) || key === "__host") continue;
    const value = globalThis[key];
    const contract = Object.hasOwn(declared, key) && declared[key];
    if (contract.kind === "class" && typeof value === "function" && value.prototype) {
      const parent = Object.getPrototypeOf(value.prototype);
      const parentName = parent === Object.prototype ? null : parent?.constructor?.name;
      let constructible = true;
      if (contract.constructible) {
        Reflect.construct(function () {}, [], value);
      } else {
        try {
          new value();
        } catch (error) {
          if (!(error instanceof TypeError)) throw error;
          constructible = false;
        }
      }
      surface[key] = {
        kind: "class",
        constructible,
        parent: parentName,
        static: memberNames(value, ["name", "length", "prototype"]),
        prototype: memberNames(value.prototype, ["constructor"]),
      };
    } else if (typeof value === "function") {
      surface[key] = { kind: "function" };
    } else if (typeof value === "object" && value !== null) {
      surface[key] = { kind: "object", own: memberNames(value, []) };
    } else {
      surface[key] = { kind: typeof value };
    }
  }
  return surface;
}

function compareSurface(profile, declared, installed) {
  assert.deepEqual(Object.keys(installed).sort(), Object.keys(declared).sort(), `${profile}: global names`);
  for (const [name, expected] of Object.entries(declared)) {
    const actual = installed[name];
    if (expected.kind === "class" && actual.kind === "class") {
      // An inherited method can also have a more specific TypeScript overload
      // on the subclass (WebSocket.addEventListener).
      for (const key of Object.keys(expected.prototype)) {
        let parent = expected.parent;
        while (parent && !Object.hasOwn(actual.prototype, key)) {
          const shape = installed[parent];
          if (Object.hasOwn(shape.prototype, key)) actual.prototype[key] = shape.prototype[key];
          parent = shape.parent;
        }
      }
    }
    assert.deepEqual(actual, expected, `${profile}: ${name}`);
  }
}

const profiles = [
  { name: "legacy", http: true, socket: true, streaming: true },
  { name: "portable", http: true, socket: true },
  { name: "portable-http", http: true },
  { name: "portable-socket", socket: true },
  { name: "portable-offline" },
];

for (const { name, http, socket, streaming } of profiles) {
  const declarations = rust(name, "declarations");
  const source = `${fixture}
    ${streaming ? 'if (new AbortSignal().aborted) throw new Error("legacy signal is already aborted");' : `
    function checkSignalConstructor() {
      // @ts-expect-error Portable signals are constructed by AbortController or AbortSignal.abort.
      new AbortSignal();
    }`}
    async function useTransports(request: Request) {
      ${http ? 'const response: Response = await fetch(request); void response;' : '// @ts-expect-error HTTP is disabled.\nawait fetch(request);'}
      ${streaming ? 'const reader: ReadableStreamDefaultReader<Uint8Array> = (await fetchStream(request)).getReader(); reader.releaseLock();' : '// @ts-expect-error fetchStream belongs to legacy.\nawait fetchStream(request);'}
      ${socket ? 'const socket = new WebSocket("wss://example.test/socket"); socket.addEventListener("message", event => console.log(event.data)); socket.addEventListener("close", event => console.log(event.code)); socket.close();' : '// @ts-expect-error WebSocket is disabled.\nnew WebSocket("wss://example.test/socket");'}
    }
  `;
  const declared = declaredSurface(compile(name, declarations, source));
  const javascript = ts.transpileModule(source, { compilerOptions: { target: options.target } }).outputText;
  const installed = JSON.parse(rust(name, "inspect", `
    const installed = (${inspectSurface})(baseline, ${JSON.stringify(declared)});
    ${javascript}
    return JSON.stringify(installed);
  `));
  compareSurface(name, declared, installed);
  process.stdout.write(`${name}: standard library contract passed\n`);
}
