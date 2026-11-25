import { useEffect, useState } from "react";
import { getSetupStatus, logPath, revealPath, type MediaSetupStatus } from "../ipc/api";
import { errText } from "./clipUtil";

function StatusDot({ ok }: { ok: boolean }) {
  return <span className={`status-dot${ok ? " ok" : ""}`} aria-hidden />;
}


export function SetupPanel({
  onClose,
}: {
  onClose: () => void;
  /** Open scrolled to this row (the model chip with no key: "models"). */
  focus?: "models";
  /** A message for the Models row (why the chip opened Setup). */
  notice?: string | null;
}) {
  const [status, setStatus] = useState<MediaSetupStatus | null>(null);
  const [statusFailed, setStatusFailed] = useState(false);
  // The app log's file; null in the browser build (the row is hidden).
  const [logFile, setLogFile] = useState<string | null>(null);
  const [logError, setLogError] = useState<string | null>(null);

  useEffect(() => {
    void logPath().then(setLogFile).catch(() => setLogFile(null));
  }, []);

  const refresh = () => {
    void getSetupStatus().then(
      (next) => {
        setStatus(next);
        setStatusFailed(false);
      },
      () => setStatusFailed(true),
    );
  };

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 5000); // picks up installs done outside the app
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => {
      clearInterval(t);
      window.removeEventListener("keydown", onKey);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="setup-backdrop" onClick={onClose}>
      <div className="setup-panel card" onClick={(e) => e.stopPropagation()}>
        <div className="setup-header">
          <h2>Setup</h2>
          <button className="icon-btn" title="Close" onClick={onClose}>✕</button>
        </div>

        {statusFailed && (
          <p className="setup-hint">
            <span role="status">Couldn’t check the media tools.</span>{" "}
            <button className="ghost-btn" onClick={refresh}>Retry</button>
          </p>
        )}
        {!status ? (
          !statusFailed && <p className="setup-hint" role="status">Checking your toolchain…</p>
        ) : (
          <>
            {/* ---- media engine ---- */}
            <section className="setup-row">
              <div className="setup-row-head">
                <StatusDot ok={status.ffmpeg} />
                <strong>Media engine</strong>
                <span className="setup-sub">ffmpeg — import, waveforms, thumbnails, MP4 export</span>
              </div>
              <p className="setup-detail">FFmpeg: {status.ffmpeg_path ?? "Not found"}</p>
              <p className="setup-detail">FFprobe: {status.ffprobe_path ?? "Not found"}</p>
              {!status.ffmpeg && (
                <p className="setup-detail warn">
                  Install both with <code>brew install ffmpeg</code>.
                </p>
              )}
            </section>

            {/* ---- DaVinci Resolve hand-off ---- */}
            <section className="setup-row">
              <div className="setup-row-head">
                <strong>DaVinci Resolve</strong>
                {status.resolve_installed !== null && (
                  <span className="setup-sub">
                    {status.resolve_installed ? "Installed" : "Not found in Applications"}
                  </span>
                )}
              </div>
              <p className="setup-detail">
                Export a DaVinci Resolve timeline from the editor, then import the XML as a new timeline in Resolve.
              </p>
            </section>

            {/* ---- app log ---- */}
            {logFile && (
              <section className="setup-row">
                <div className="setup-row-head">
                  <StatusDot ok />
                  <strong>Logs</strong>
                  <span className="setup-sub">Media processing errors</span>
                </div>
                <p className="setup-detail">
                  <code>{logFile}</code>
                  <button className="primary-btn tier-btn" onClick={() => {
                    setLogError(null);
                    void revealPath(logFile).catch((error) => setLogError(errText(error)));
                  }}>
                    Reveal logs
                  </button>
                </p>
                {logError && <p role="alert">{logError}</p>}
              </section>
            )}

            <p className="setup-footer">
              Editing runs on this machine.
            </p>
          </>
        )}
      </div>
    </div>
  );
}
