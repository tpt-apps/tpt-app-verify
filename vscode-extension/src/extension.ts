// TPT Verify — VS Code extension.
//
// Shells out to the `tpt-verify` CLI (built from this same repo's `cli/`
// crate, `--features pro` optional) rather than re-embedding the engine:
// the CLI is already the "run this from somewhere other than a browser
// tab" distribution channel this whole extension exists to plug into an
// editor, so reusing it keeps exactly one place that knows how to invoke
// analysis and format output.
//
// Position accuracy: the CLI's JSON output (see `cli/src/json.rs`) has no
// line/column info per finding — the engine doesn't track source spans.
// Diagnostics are therefore placed on the function's own `fn name(...)`
// declaration line (found via a text search), not the exact statement —
// an honest approximation, not a claim of precision the engine doesn't
// have.

import * as vscode from "vscode";
import { execFile } from "child_process";

interface Finding {
  kind: string;
  summary: string;
  detail: string;
  explanation: string;
}

interface FunctionResult {
  name: string;
  params: string[];
  error: string | null;
  clean: boolean;
  paths_explored: number;
  findings: Finding[];
}

interface FileResult {
  path: string;
  parse_error: string | null;
  functions: FunctionResult[];
}

interface CliOutput {
  files: FileResult[];
}

let diagnostics: vscode.DiagnosticCollection;
let output: vscode.OutputChannel;
let warnedMissingBinary = false;

export function activate(context: vscode.ExtensionContext): void {
  diagnostics = vscode.languages.createDiagnosticCollection("tptVerify");
  output = vscode.window.createOutputChannel("TPT Verify");
  context.subscriptions.push(diagnostics, output);

  context.subscriptions.push(
    vscode.commands.registerCommand("tptVerify.verifyFile", () => {
      const editor = vscode.window.activeTextEditor;
      if (editor && editor.document.languageId === "rust") {
        verifyDocument(editor.document);
      } else {
        vscode.window.showInformationMessage("TPT Verify: open a .rs file first.");
      }
    })
  );

  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument((document) => {
      const runOnSave = vscode.workspace.getConfiguration("tptVerify").get<boolean>("runOnSave", true);
      if (runOnSave && document.languageId === "rust") {
        verifyDocument(document);
      }
    })
  );

  // Verify whatever Rust file is already open when the extension activates,
  // so the diagnostics aren't only ever one edit behind.
  const active = vscode.window.activeTextEditor;
  if (active && active.document.languageId === "rust") {
    verifyDocument(active.document);
  }
}

export function deactivate(): void {
  diagnostics?.dispose();
}

function verifyDocument(document: vscode.TextDocument): void {
  const exe = vscode.workspace.getConfiguration("tptVerify").get<string>("executablePath", "tpt-verify");

  execFile(exe, ["--json", document.fileName], { timeout: 10_000 }, (error, stdout, stderr) => {
    // Exit codes 0 (clean) and 1 (findings/unsupported) both mean the CLI
    // ran fine and `stdout` has real JSON to parse; only a spawn failure
    // (missing binary) or exit code 2 (usage/IO error inside the CLI
    // itself) means something actually went wrong here.
    const exitCode = (error as NodeJS.ErrnoException & { code?: number })?.code;
    const spawnFailed = error && (error as NodeJS.ErrnoException).errno !== undefined && !stdout;

    if (spawnFailed || exitCode === 2 || (error && !stdout)) {
      diagnostics.delete(document.uri);
      if (!warnedMissingBinary) {
        warnedMissingBinary = true;
        vscode.window
          .showWarningMessage(
            `TPT Verify: couldn't run '${exe}' (${error?.message ?? "unknown error"}). Set "tptVerify.executablePath" ` +
              "to the built tpt-verify binary, e.g. the workspace's target/debug/tpt-verify(.exe).",
            "Open Settings"
          )
          .then((choice) => {
            if (choice === "Open Settings") {
              vscode.commands.executeCommand("workbench.action.openSettings", "tptVerify.executablePath");
            }
          });
      }
      output.appendLine(`[error] ${error?.message ?? "unknown"}\n${stderr}`);
      return;
    }

    let parsed: CliOutput;
    try {
      parsed = JSON.parse(stdout);
    } catch (e) {
      output.appendLine(`[error] couldn't parse tpt-verify --json output: ${e}\n${stdout}`);
      return;
    }

    const file = parsed.files.find((f) => samePath(f.path, document.fileName)) ?? parsed.files[0];
    if (!file) {
      diagnostics.delete(document.uri);
      return;
    }

    diagnostics.set(document.uri, buildDiagnostics(document, file));
  });
}

function samePath(a: string, b: string): boolean {
  const norm = (p: string) => p.replace(/\\/g, "/").toLowerCase();
  return norm(a) === norm(b);
}

function buildDiagnostics(document: vscode.TextDocument, file: FileResult): vscode.Diagnostic[] {
  const text = document.getText();
  const out: vscode.Diagnostic[] = [];

  if (file.parse_error) {
    out.push(new vscode.Diagnostic(new vscode.Range(0, 0, 0, 0), `TPT Verify: ${file.parse_error}`, vscode.DiagnosticSeverity.Error));
    return out;
  }

  for (const fn of file.functions) {
    const range = functionDeclRange(document, text, fn.name);

    if (fn.error) {
      out.push(new vscode.Diagnostic(range, `TPT Verify: not analyzed — ${fn.error}`, vscode.DiagnosticSeverity.Hint));
      continue;
    }

    for (const finding of fn.findings) {
      const severity = finding.kind === "RangeOverflow" ? vscode.DiagnosticSeverity.Information : vscode.DiagnosticSeverity.Warning;
      const diagnostic = new vscode.Diagnostic(range, `TPT Verify [${finding.kind}]: ${finding.summary} — ${finding.explanation}`, severity);
      diagnostic.source = "tpt-verify";
      out.push(diagnostic);
    }
  }

  return out;
}

/// Finds `fn <name>(` in the document and returns a `Range` covering that
/// line — the best available anchor without span info from the engine
/// (see this file's module doc).
function functionDeclRange(document: vscode.TextDocument, text: string, name: string): vscode.Range {
  const pattern = new RegExp(`\\bfn\\s+${escapeRegExp(name)}\\s*\\(`);
  const match = pattern.exec(text);
  if (!match) {
    return new vscode.Range(0, 0, 0, 0);
  }
  const startPos = document.positionAt(match.index);
  const line = document.lineAt(startPos.line);
  return new vscode.Range(startPos.line, 0, startPos.line, line.text.length);
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
