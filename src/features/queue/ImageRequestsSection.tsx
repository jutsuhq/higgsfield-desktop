import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";

import { AlertDialog } from "../../app/Dialog";
import { commands, type ImageRequestRow_Serialize } from "../../bindings";
import { SectionLabel } from "../../design/Components";
import { Button } from "../../design/Styles";
import { useSetCreateSide } from "../create/imageData";
import { RequestRow } from "../create/RequestRow";

/**
 * Image requests from the Image side of Create (`ImageRequestsSection`): they run beside the job queue, so they are
 * listed on their own, with Cancel while they run and Review… while an agent's request waits for the person. Their
 * images are on the Image side of Create and in Library › Images.
 */
export function ImageRequestsSection({ requests }: { requests: ImageRequestRow_Serialize[] }) {
  const navigate = useNavigate();
  const setCreateSide = useSetCreateSide();
  const [reviewing, setReviewing] = useState<ImageRequestRow_Serialize | null>(null);
  if (requests.length === 0) return null;
  return (
    <>
      <div className="flex items-baseline gap-medium pt-18">
        <SectionLabel title="Images" detail={String(requests.length)} />
        <Button
          variant="textLink"
          title="Show in Create"
          hint="Show images in Create"
          onClick={() => {
            setCreateSide("image");
            void navigate({ to: "/create" });
          }}
        />
      </div>
      <div role="list" aria-label="Image requests" className="flex flex-col gap-small">
        {requests.map((request) => (
          <div role="listitem" key={request.id}>
            <RequestRow
              row={request}
              onCancel={() => void commands.imagesCancel(request.id)}
              onReview={() => {
                setReviewing(request);
              }}
            />
          </div>
        ))}
      </div>
      {reviewing !== null && (
        <AlertDialog
          title="Send this agent's request?"
          message={reviewing.review ?? ""}
          onCancel={() => {
            setReviewing(null);
          }}
        >
          <Button
            variant="secondary"
            title="Not Now"
            onClick={() => {
              setReviewing(null);
            }}
          />
          <Button
            variant="secondary"
            title="Cancel Request"
            destructive
            onClick={() => {
              void commands.imagesCancel(reviewing.id);
              setReviewing(null);
            }}
          />
          <Button
            variant="primary"
            title="Send"
            onClick={() => {
              void commands.agentsSendHeldImage(reviewing.id);
              setReviewing(null);
            }}
          />
        </AlertDialog>
      )}
    </>
  );
}
