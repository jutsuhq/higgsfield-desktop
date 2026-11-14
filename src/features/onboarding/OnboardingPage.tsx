import { Check, CircleUser, KeyRound, ShieldCheck, User, type LucideIcon } from "lucide-react";
import { useId, useState, type ReactNode } from "react";

import { useConnection } from "../../app/liveData";
import { commands } from "../../bindings";
import { GlowingAppIcon, Pill, SignatureBackdrop } from "../../design/Components";
import { Spinner } from "../../design/Spinner";
import { Button, Card, TextField } from "../../design/Styles";
import { useCommandMutation } from "../../lib/ipc";

/** First launch (OnboardingView.swift): two ways to connect, side by side, under the centered hero. */
export function OnboardingPage() {
  return (
    <div className="relative flex h-full flex-col overflow-auto bg-background">
      <SignatureBackdrop />
      <div className="relative mx-auto flex w-full max-w-820 flex-1 flex-col items-center justify-center px-page py-40 text-center">
        <div className="flex flex-col items-center gap-small pb-36">
          <div className="pb-large">
            <GlowingAppIcon />
          </div>
          <p className="relative m-0 text-caption font-bold tracking-hero text-faint uppercase">
            Welcome to Jutsu Studio
          </p>
          <h1 className="relative m-0 font-display text-hero font-heavy text-text uppercase">
            Connect Higgsfield
          </h1>
          <p className="relative m-0 text-button text-muted">
            Pick one way to connect. You can switch later in Settings.
          </p>
        </div>
        <div className="relative grid w-full grid-cols-1 gap-large text-left sm:grid-cols-2">
          <AccountCard />
          <ApiKeyCard />
          <OrBadge />
        </div>
        <p className="relative m-0 flex items-start gap-6 pt-x-large text-caption-plus text-faint">
          <ShieldCheck aria-hidden size={13} className="mt-1 shrink-0" />
          <span>
            Jutsu Studio talks to Higgsfield directly. No account of ours, no telemetry. Unofficial, not
            affiliated with Higgsfield AI.
          </span>
        </p>
      </div>
    </div>
  );
}

/** "or" on the seam between the two cards. */
function OrBadge() {
  return (
    <span
      aria-hidden
      className="absolute top-1/2 left-1/2 hidden size-32 -translate-1/2 items-center justify-center rounded-full border border-stroke bg-background text-caption font-bold text-muted uppercase sm:flex"
    >
      or
    </span>
  );
}

/** One way to connect (`.acct`): icon, title and a pill, then the content. */
function ConnectionCard({
  title,
  icon: Icon,
  badge,
  children,
}: {
  title: string;
  icon: LucideIcon;
  badge: string;
  children: ReactNode;
}) {
  return (
    <Card className="flex flex-col gap-14 p-24">
      <div className="flex items-center gap-medium">
        <span
          aria-hidden
          className="flex size-36 shrink-0 items-center justify-center rounded-tile border-[0.5px] border-glass-stroke bg-glass text-text"
        >
          <Icon size={15} strokeWidth={2.25} />
        </span>
        <h2 className="m-0 mr-auto text-heading font-bold whitespace-nowrap text-text">{title}</h2>
        <Pill text={badge} />
      </div>
      {children}
    </Card>
  );
}

/** A checkmarked line in a connection card. */
function Benefit({ text }: { text: string }) {
  return (
    <li className="flex items-center gap-8 text-body text-text">
      <Check aria-hidden size={11} strokeWidth={3.5} className="shrink-0 text-accent" />
      {text}
    </li>
  );
}

function AccountCard() {
  const isIdle = useConnection()?.activity.kind === "idle";
  const signIn = useCommandMutation(() => commands.startSignIn());
  return (
    <ConnectionCard title="Higgsfield account" icon={User} badge="Subscription credits">
      <p className="m-0 text-body text-muted">
        Spend the credits of your higgsfield.ai plan. You sign in on Higgsfield&apos;s own page through its
        MCP server, so Jutsu Studio never sees your password.
      </p>
      <ul className="m-0 flex list-none flex-col gap-small p-0 pt-x-small">
        <Benefit text="Uses the credits you already pay for" />
        <Benefit text="Nothing to copy or paste" />
        <Benefit text="Your password stays on higgsfield.ai" />
      </ul>
      <div className="min-h-large flex-1" />
      <Button
        variant="primary"
        title="Connect your Higgsfield account"
        icon={CircleUser}
        className="w-full"
        disabled={!isIdle}
        onClick={() => {
          signIn.mutate();
        }}
      />
    </ConnectionCard>
  );
}

function ApiKeyCard() {
  const connection = useConnection();
  const isIdle = connection?.activity.kind === "idle";
  const isValidating = connection?.activity.kind === "validatingKey";
  const [keyId, setKeyId] = useState("");
  const [secret, setSecret] = useState("");
  const secretId = useId();
  const connect = useCommandMutation((form: { keyId: string; secret: string }) =>
    commands.connectApiKey(form.keyId, form.secret),
  );
  // Mirrors `ApiKeyForm::is_complete`; Rust checks again and answers an incomplete key with its own alert.
  const isComplete = keyId.trim() !== "" && secret.trim() !== "";
  const submit = () => {
    if (!isComplete || !isIdle) return;
    connect.mutate({ keyId, secret });
  };

  return (
    <ConnectionCard title="API key" icon={KeyRound} badge="Pay as you go">
      <p className="m-0 text-body text-muted">
        Billed per second of video from a prepaid balance in the Higgsfield console. The key is kept in{" "}
        {connection?.credentialStore ?? "the system keyring"}.
      </p>
      <div className="flex flex-col gap-small pt-x-small">
        <TextField
          aria-label="Key ID"
          placeholder="Key ID"
          autoComplete="off"
          spellCheck={false}
          value={keyId}
          onChange={(event) => {
            setKeyId(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key !== "Enter") return;
            event.preventDefault();
            document.getElementById(secretId)?.focus();
          }}
        />
        <TextField
          id={secretId}
          aria-label="Secret"
          placeholder="Secret"
          type="password"
          autoComplete="off"
          value={secret}
          onChange={(event) => {
            setSecret(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key !== "Enter") return;
            event.preventDefault();
            submit();
          }}
        />
      </div>
      <div className="min-h-large flex-1" />
      {/* While the key is checked, the system spinner sits before the label, as in OnboardingView.swift. */}
      <Button
        variant="secondary"
        title="Connect key"
        icon={KeyRound}
        leading={isValidating ? <Spinner /> : undefined}
        className="w-full"
        disabled={!isComplete || !isIdle}
        onClick={submit}
      />
      <Button
        variant="textLink"
        title="Create a key in the Higgsfield console"
        className="self-start"
        onClick={() => {
          void commands.openHiggsfieldConsole();
        }}
      />
    </ConnectionCard>
  );
}
