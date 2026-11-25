import { useState } from "react";
import { useSetPreferences } from "../ipc/queries";
import { errText } from "./clipUtil";

const LISTS = {
  fillers: { label: "Custom fillers", file: "custom-fillers.txt", key: "custom_filler_words" },
  profanity: { label: "Profanity list", file: "profanity-phrases.txt", key: "profanity_phrases" },
  keywords: { label: "Keyword list", file: "keyword-phrases.txt", key: "keyword_phrases" },
  exceptions: { label: "Never cut", file: "never-cut-phrases.txt", key: "never_cut_phrases" },
} as const;

function parsePhrases(text: string): string[] {
  const normalized = text.replace(/\r\n?/g, "\n");
  const entries = [...new Set(normalized.split("\n").map((line) => line.trim()).filter(Boolean))];
  const characters = entries.reduce((total, line) => total + Array.from(line).length, 0);
  if (entries.length > 200 || characters > 4000 || /[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/.test(normalized)) {
    throw new Error("Use up to 200 phrases and 4,000 characters, with one phrase per line.");
  }
  return entries;
}

export function CustomFillers({ phrases, kind }: { phrases: string[]; kind: keyof typeof LISTS }) {
  const list = LISTS[kind];
  const [draft, setDraft] = useState<string | null>(null);
  const [fileError, setFileError] = useState("");
  const [importing, setImporting] = useState(false);
  const save = useSetPreferences();

  const importList = async (file: File) => {
    setFileError("");
    setImporting(true);
    try {
      if (file.size > 32 * 1024) throw new Error("Choose a phrase list under 32 KiB.");
      let text = new TextDecoder("utf-8", { fatal: true }).decode(await file.arrayBuffer());
      if (file.name.toLowerCase().endsWith(".json")) {
        const entries: unknown = JSON.parse(text);
        if (!Array.isArray(entries) || entries.some((entry) => typeof entry !== "string" || /[\r\n]/.test(entry))) {
          throw new Error("Use a JSON array with one phrase per string.");
        }
        text = entries.join("\n");
      }
      setDraft(parsePhrases(text).join("\n"));
    } catch (error) {
      setFileError(errText(error));
    } finally {
      setImporting(false);
    }
  };

  const exportList = (format: "text" | "json") => {
    const json = format === "json";
    const content = json ? JSON.stringify(phrases, null, 2) : phrases.join("\n");
    const type = json ? "application/json" : "text/plain;charset=utf-8";
    const url = URL.createObjectURL(new Blob([content], { type }));
    const link = document.createElement("a");
    link.href = url;
    link.download = json ? list.file.replace(/\.txt$/, ".json") : list.file;
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };

  return (
    <details>
      <summary>{list.label}</summary>
      <form onSubmit={(event) => {
        event.preventDefault();
        if (draft === null || save.isPending || importing) return;
        setFileError("");
        let entries: string[];
        try {
          entries = parsePhrases(draft);
        } catch (error) {
          setFileError(errText(error));
          return;
        }
        save.mutate({ preferences: { [list.key]: entries } }, {
          onSuccess: () => setDraft(null),
        });
      }}>
        <label>
          One phrase per line
          <textarea value={draft ?? phrases.join("\n")} maxLength={8199} disabled={save.isPending || importing}
            onChange={(event) => setDraft(event.target.value)} />
        </label>
        <button type="submit" className="ghost-btn" disabled={draft === null || save.isPending || importing}>Save list</button>
        <label>Import list <input type="file" accept=".txt,.json,text/plain,application/json" disabled={save.isPending || importing}
          onChange={(event) => {
            const file = event.currentTarget.files?.[0];
            event.currentTarget.value = "";
            if (file) void importList(file);
          }} /></label>
        <button type="button" className="ghost-btn" disabled={!phrases.length} onClick={() => exportList("text")}>Export text</button>
        <button type="button" className="ghost-btn" disabled={!phrases.length} onClick={() => exportList("json")}>Export JSON</button>
        {fileError && <p role="alert">{fileError}</p>}
        {save.isError && <p role="alert">{errText(save.error)}</p>}
      </form>
    </details>
  );
}
