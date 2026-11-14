import { Check, Compass, Copy } from "lucide-react";
import { useState } from "react";

import { Dialog } from "../../app/Dialog";
import { useConnection } from "../../app/liveData";
import { commands } from "../../bindings";
import { Spinner } from "../../design/Spinner";
import { Button } from "../../design/Styles";

/**
 * Progress while sign-in is open in the browser (SignInSheet.swift), with the page's link in case the browser
 * didn't come up. Shown over any page while the connection is signing in; Cancel and Esc stop the listener.
 */
export function SignInSheet() {
  const activity = useConnection()?.activity;
  if (activity?.kind !== "signingIn") return null;
  return <SheetBody url={activity.url} />;
}

function SheetBody({ url }: { url: string | null }) {
  const [isCopied, setIsCopied] = useState(false);
  const cancel = () => {
    void commands.cancelSignIn();
  };
  return (
    <Dialog title="Finish signing in in your browser" onCancel={cancel}>
      <div className="flex items-center gap-10">
        <Spinner />
        <p className="m-0 text-heading font-bold">Finish signing in in your browser</p>
      </div>
      <p className="m-0 text-body text-muted">
        Higgsfield&apos;s sign-in page is open. Approve access for Jutsu Studio and you&apos;ll be brought
        back here automatically.
      </p>
      <div className="flex items-center gap-small">
        {url && (
          <>
            <Button
              variant="secondary"
              title="Open Again"
              icon={Compass}
              onClick={() => {
                void commands.openSignInPage();
              }}
            />
            <Button
              variant="secondary"
              title={isCopied ? "Copied" : "Copy Link"}
              icon={isCopied ? Check : Copy}
              hint="Copy sign-in link"
              onClick={() => {
                void navigator.clipboard.writeText(url).then(() => {
                  setIsCopied(true);
                });
              }}
            />
          </>
        )}
        <span className="flex-1" />
        <Button variant="secondary" title="Cancel" onClick={cancel} />
      </div>
    </Dialog>
  );
}
