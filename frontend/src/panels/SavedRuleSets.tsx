import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { queryKeys, useSetPreferences } from "../ipc/queries";
import type { Preferences } from "../ipc/types";
import { errText } from "./clipUtil";

export function SavedRuleSets({ preferences }: { preferences: Preferences }) {
  const [name, setName] = useState("");
  const [selected, setSelected] = useState("");
  const [notice, setNotice] = useState("");
  const save = useSetPreferences();
  const queryClient = useQueryClient();
  const sets = preferences.saved_rule_sets ?? [];
  const active = sets.find((set) => set.name === selected);

  const persist = (patch: Partial<Preferences>, message: string, after?: () => void) => {
    setNotice("");
    save.mutate({ preferences: patch, expected_rule_sets: patch.saved_rule_sets ? sets : undefined }, {
      onSuccess: (updated) => {
        queryClient.setQueryData(queryKeys.preferences, updated);
        setNotice(message);
        after?.();
      },
      onError: () => void queryClient.invalidateQueries({ queryKey: queryKeys.preferences }),
    });
  };

  return (
    <details>
      <summary>Rule sets</summary>
      <form onSubmit={(event) => {
        event.preventDefault();
        const label = name.trim();
        if (!label || save.isPending) return;
        persist({
          saved_rule_sets: [...sets, {
            name: label,
            custom_filler_words: preferences.custom_filler_words,
            profanity_phrases: preferences.profanity_phrases ?? [],
            keyword_phrases: preferences.keyword_phrases ?? [],
            never_cut_phrases: preferences.never_cut_phrases ?? [],
          }],
        }, "Rule set saved", () => {
          setSelected(label);
          setName("");
        });
      }}>
        <label>Name <input value={name} maxLength={80} disabled={save.isPending}
          onChange={(event) => setName(event.target.value)} /></label>
        <button type="submit" className="ghost-btn" disabled={!name.trim() || save.isPending}>Save current lists</button>
      </form>
      {sets.length > 0 && (
        <div>
          <label>Saved set <select value={active?.name ?? ""} disabled={save.isPending}
            onChange={(event) => setSelected(event.target.value)}>
            <option value="">Choose a set</option>
            {sets.map((set) => <option key={set.name} value={set.name}>{set.name}</option>)}
          </select></label>
          <button type="button" className="ghost-btn" disabled={!active || save.isPending} onClick={() => {
            if (!active) return;
            persist({
              custom_filler_words: active.custom_filler_words,
              profanity_phrases: active.profanity_phrases,
              keyword_phrases: active.keyword_phrases,
              never_cut_phrases: active.never_cut_phrases,
            }, "Rule set loaded");
          }}>Load lists</button>
          <button type="button" className="ghost-btn" disabled={!active || save.isPending} onClick={() => {
            if (!active) return;
            persist({ saved_rule_sets: sets.filter((set) => set.name !== active.name) }, "Rule set removed", () => setSelected(""));
          }}>Remove set</button>
        </div>
      )}
      {notice && <p role="status">{notice}</p>}
      {save.isError && <p role="alert">{errText(save.error)}</p>}
    </details>
  );
}
