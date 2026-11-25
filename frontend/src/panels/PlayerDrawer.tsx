import { useStore } from "@tanstack/react-store";
import { toggleShortsDrawer, viewStore } from "../state/viewStore";
import { ChatPanel } from "./ChatPanel";

export function PlayerDrawer({ projectId }: { projectId: string }) {
  const open = useStore(viewStore, (view) => view.shortsDrawerOpen);
  return (
    <div className={`shorts-drawer${open ? " open" : ""}`}>
      <div className="shorts-drawer-bar">
        <button
          className="shorts-drawer-toggle"
          onClick={toggleShortsDrawer}
          aria-expanded={open}
        >
          {open ? "Hide chat" : "Show chat"}
        </button>
      </div>
      {open && <ChatPanel projectId={projectId} />}
    </div>
  );
}
