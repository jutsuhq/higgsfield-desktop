import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";

import { Dialog } from "../../app/Dialog";
import { commands } from "../../bindings";
import appIcon from "../../design/assets/app-icon.png";
import { Button } from "../../design/Styles";
import { useCommand } from "../../lib/ipc";
import { queryKeys } from "../../lib/queryKeys";

/** The window is as tall as what it shows (`.fixedSize(horizontal: false, vertical: true)`). */
function useFitWindow(content: HTMLElement | null, isReady: boolean) {
  useEffect(() => {
    if (!content || !isReady) return;
    try {
      void getCurrentWindow()
        .setSize(new LogicalSize(content.offsetWidth, content.offsetHeight))
        .catch(() => undefined);
    } catch {
      // Outside the app's own window (a test) there is nothing to fit.
    }
  }, [content, isReady]);
}

/**
 * The About window (AboutView.swift): the icon and name on the signature gradient, one of the brand moments that
 * carries it; then the version, the disclaimer, the license and the credits, the repository, and the third-party
 * notices this build's installers need.
 */
export function AboutPage() {
  const about = useCommand(queryKeys.about, () => commands.aboutContent()).data;
  const [content, setContent] = useState<HTMLElement | null>(null);
  const [isShowingNotices, setIsShowingNotices] = useState(false);
  useFitWindow(content, about !== undefined);

  return (
    <div ref={setContent} className="flex w-340 flex-col bg-background text-text">
      <div className="flex flex-col items-center gap-medium bg-(image:--about-hero) pt-44 pb-x-large">
        <img src={appIcon} alt="" aria-hidden className="size-96 rounded-app-icon shadow-about-icon" />
        <h1 className="m-0 font-display text-stage font-heavy tracking-about text-text uppercase [text-shadow:var(--about-name-shadow)]">
          Jutsu Studio
        </h1>
      </div>
      {about && (
        <div className="flex flex-col items-center gap-small px-x-large pt-small pb-x-large text-center">
          <p className="m-0 text-small font-medium text-muted tabular-nums select-text">{about.version}</p>
          <p className="m-0 text-small font-semibold text-text">{about.disclaimer}</p>
          <div className="flex flex-col gap-2 text-caption text-faint">
            {about.credits.map((line) => (
              <p key={line} className="m-0">
                {line}
              </p>
            ))}
          </div>
          <div className="flex flex-col items-center gap-2 pt-x-small">
            <Button
              variant="textLink"
              title={about.repositoryTitle}
              onClick={() => void commands.aboutOpenRepository().catch(() => undefined)}
            />
            <Button
              variant="textLink"
              title={about.noticesTitle}
              onClick={() => {
                setIsShowingNotices(true);
              }}
            />
          </div>
        </div>
      )}
      {about && isShowingNotices && (
        <Dialog
          title={about.noticesTitle}
          onCancel={() => {
            setIsShowingNotices(false);
          }}
        >
          <pre className="m-0 max-h-[60vh] overflow-auto font-sans text-caption whitespace-pre-wrap text-muted select-text">
            {about.notices}
          </pre>
          <div className="flex justify-end">
            <Button
              variant="primary"
              title="Done"
              onClick={() => {
                setIsShowingNotices(false);
              }}
            />
          </div>
        </Dialog>
      )}
    </div>
  );
}
