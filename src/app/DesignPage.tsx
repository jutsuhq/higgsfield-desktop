import { CircleCheck, Eye, Film, Key, Layers, Plus, Sparkles, Star, Trash2, X } from "lucide-react";
import { useState, type ReactNode } from "react";

import {
  BrandMark,
  DropZone,
  EmptyState,
  EstimatedProgress,
  FreshGlow,
  GlowingAppIcon,
  MediaTag,
  PageHeader,
  Pill,
  PriceTag,
  RenderProgress,
  SectionLabel,
  SidebarRow,
  SignatureBackdrop,
  type ColorRole,
} from "../design/Components";
import { FlowLayout } from "../design/FlowLayout";
import { HoverHint } from "../design/HoverHint";
import { PagedFeedFooter } from "../design/PagedFeedFooter";
import { FilterMenu, FilterToggle, SearchField, SegmentedControl } from "../design/Pickers";
import { Spinner } from "../design/Spinner";
import { Button, Card, TextField, Well, type ButtonVariant } from "../design/Styles";
import { ValueStepper } from "../design/ValueStepper";
import { AccountPlate, PlanBadge } from "./AccountPlate";
import type { AccountPlateModel, PlanTier } from "./liveData";
import { Toast, ToastHost } from "./ToastHost";
import { useToastStore } from "./toastStore";

/** Development only: every shared control in every state, for comparing against the macOS app. */
export function DesignPage() {
  return (
    <div className="relative h-full bg-background text-text">
      <div className="flex h-full flex-col gap-x-large overflow-auto px-page pt-page-top pb-page">
        <PageHeader title="Design" subtitle="Every control in every state." />
        <Buttons />
        <Surfaces />
        <Capsules />
        <Navigation />
        <Progress />
        <Pickers />
        <Account />
        <Feedback />
      </div>
      <ToastHost />
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-medium">
      <SectionLabel title={title} />
      {children}
    </section>
  );
}

/** One state of a control with its name under it. */
function State({ name, children }: { name: string; children: ReactNode }) {
  return (
    <div className="flex flex-col items-start gap-6">
      {children}
      <span className="text-caption text-faint">{name}</span>
    </div>
  );
}

const forced = {
  Default: {},
  Hover: { "data-hover": true },
  Pressed: { "data-hover": true, "data-pressed": true },
  Focused: { "data-focus": true },
} as const;

function ButtonRow({
  variant,
  title,
  icon,
  price,
}: {
  variant: ButtonVariant;
  title: string;
  icon?: typeof Plus;
  price?: string;
}) {
  return (
    <FlowLayout spacing="small" className="items-end gap-x-large">
      {Object.entries(forced).map(([name, attributes]) => (
        <State key={name} name={name}>
          <Button variant={variant} title={title} icon={icon} price={price} {...attributes} />
        </State>
      ))}
      {variant !== "textLink" && (
        <State name="Disabled">
          <Button variant={variant} title={title} icon={icon} price={price} disabled />
        </State>
      )}
      {(variant === "secondary" || variant === "icon") && (
        <State name="Destructive">
          <Button variant={variant} title="Delete" icon={variant === "icon" ? Trash2 : icon} destructive />
        </State>
      )}
    </FlowLayout>
  );
}

function Buttons() {
  return (
    <Section title="Buttons">
      <ButtonRow variant="primary" title="Generate" icon={Sparkles} price="15 cr" />
      <ButtonRow variant="primary" title="Connect your Higgsfield account" />
      <ButtonRow variant="secondary" title="New job" icon={Plus} />
      <ButtonRow variant="icon" title="Dismiss" icon={X} />
      <ButtonRow variant="textLink" title="Create a key in the Higgsfield console" />
    </Section>
  );
}

function Surfaces() {
  return (
    <Section title="Surfaces and fields">
      <FlowLayout spacing="small" className="gap-x-large">
        <State name="Card">
          <Card className="h-80 w-200" />
        </State>
        <State name="Card, highlighted">
          <Card isHighlighted className="h-80 w-200" />
        </State>
        <State name="Well">
          <Well className="h-80 w-200" />
        </State>
        <State name="Drop zone">
          <DropZone className="h-80 w-200" />
        </State>
        <State name="Drop zone, lit">
          <DropZone isHighlighted className="h-80 w-200" />
        </State>
      </FlowLayout>
      <FlowLayout spacing="small" className="gap-x-large">
        <State name="Field">
          <TextField placeholder="Key ID" className="w-240" />
        </State>
        <State name="Field, focused">
          <TextField placeholder="Secret" data-focus className="w-240" />
        </State>
        <State name="Field, disabled">
          <TextField placeholder="Key ID" disabled className="w-240" />
        </State>
      </FlowLayout>
    </Section>
  );
}

const roles: ColorRole[] = ["accent", "info", "warning", "danger", "faint"];

function Capsules() {
  return (
    <Section title="Pills and tags">
      <FlowLayout>
        <Pill text="Subscription credits" />
        {roles.map((role) => (
          <Pill key={role} text={role} dot={role} />
        ))}
        <Pill text="1 of 2 slots busy" dot="accent" isPulsing />
      </FlowLayout>
      <FlowLayout>
        <span className="rounded-full bg-accent text-ink">
          <PriceTag text="15 cr" />
        </span>
        <MediaTag text="Motion" />
        <MediaTag text="Draft" />
        <MediaTag text="0:05" />
      </FlowLayout>
    </Section>
  );
}

function Navigation() {
  return (
    <Section title="Shell">
      <div className="flex flex-wrap items-start gap-x-large">
        <div className="flex w-(--sidebar-width) flex-col gap-2 rounded-field border border-stroke p-10">
          <div className="px-6 pb-18">
            <BrandMark />
          </div>
          <SidebarRow title="Create" icon={Sparkles} isSelected />
          <SidebarRow title="Queue" icon={Layers} isSelected={false} badge={3} />
          <SidebarRow title="Library" icon={Film} isSelected={false} data-hover />
          <SidebarRow title="Library" icon={Film} isSelected={false} />
          <SidebarRow title="Create" icon={Sparkles} isSelected isFocused />
        </div>
        <div className="relative flex h-280 w-360 items-center justify-center overflow-hidden rounded-card border border-stroke">
          <SignatureBackdrop />
          <GlowingAppIcon />
        </div>
        <div className="flex min-w-360 flex-1 flex-col">
          <PageHeader
            title="Queue"
            subtitle="Jobs keep running in the background and resume after restart."
            trailing={<Button variant="secondary" icon={Plus} title="New job" />}
          />
          <SectionLabel title="Finished" detail="8" />
          <Card className="mt-small">
            <EmptyState
              icon={Layers}
              title="Nothing running"
              detail="Start a draft from Create. Jobs keep running when the window is closed."
              action={<Button variant="secondary" title="Go to Create" />}
            />
          </Card>
          <EmptyState icon={Film} title="No results yet" />
        </div>
      </div>
    </Section>
  );
}

function Progress() {
  const [glow, setGlow] = useState(0);
  return (
    <Section title="Progress">
      <div className="grid max-w-640 grid-cols-2 gap-x-large gap-y-medium">
        <State name="0%">
          <RenderProgress value={0} />
        </State>
        <State name="42%">
          <RenderProgress value={0.42} />
        </State>
        <State name="100%">
          <RenderProgress value={1} />
        </State>
        <State name="Estimate">
          <EstimatedProgress
            estimate={{
              label: "usually ~2 min",
              fraction: 0.35,
              isOverdue: false,
              help: "From your last 5 jobs",
            }}
          />
        </State>
        <State name="Estimate, overdue">
          <EstimatedProgress
            estimate={{
              label: "taking longer than usual",
              fraction: 1,
              isOverdue: true,
              help: "From your last 5 jobs",
            }}
          />
        </State>
        <State name="Spinner">
          <Spinner />
        </State>
      </div>
      <FlowLayout spacing="small" className="items-center gap-x-large">
        <FreshGlow key={glow} isFresh>
          <Card className="h-80 w-200" />
        </FreshGlow>
        <Button
          variant="secondary"
          title="Replay done glow"
          onClick={() => {
            setGlow((count) => count + 1);
          }}
        />
      </FlowLayout>
    </Section>
  );
}

function Pickers() {
  const [kind, setKind] = useState("video");
  const [tab, setTab] = useState("general");
  const [mode, setMode] = useState<string | null>(null);
  const [sort, setSort] = useState("newest");
  const [starred, setStarred] = useState(false);
  const [query, setQuery] = useState("");
  const [variants, setVariants] = useState(1);
  return (
    <Section title="Pickers">
      <FlowLayout spacing="small" className="items-end gap-x-large">
        <State name="Segmented, compact">
          <SegmentedControl
            isCompact
            value={kind}
            onChange={setKind}
            options={[
              { value: "video", title: "Video" },
              { value: "image", title: "Image" },
            ]}
          />
        </State>
        <div className="w-360">
          <State name="Segmented, shared width">
            <SegmentedControl
              value={tab}
              onChange={setTab}
              options={[
                { value: "connection", title: "Connection" },
                { value: "general", title: "General" },
                { value: "output", title: "Output" },
              ]}
            />
          </State>
        </div>
      </FlowLayout>
      <FlowLayout spacing="small" className="items-end gap-x-large">
        <State name="Filter, any">
          <FilterMenu
            title="Mode"
            value={mode}
            onChange={setMode}
            options={[
              { value: null, title: "Any mode" },
              { value: "motion", title: "Motion" },
              { value: "restyle", title: "Restyle" },
            ]}
          />
        </State>
        <State name="Filter, hover">
          <FilterMenu
            title="Resolution"
            value={null}
            onChange={() => undefined}
            options={[{ value: null, title: "Any resolution" }]}
            data-hover
          />
        </State>
        <State name="Filter, filtering">
          <FilterMenu
            title="Sort"
            value={sort}
            onChange={setSort}
            options={[
              { value: "oldest", title: "Oldest first" },
              { value: "newest", title: "Newest first" },
            ]}
          />
        </State>
        <State name="Toggle">
          <FilterToggle title="Starred" icon={Star} isOn={starred} onChange={setStarred} />
        </State>
        <State name="Toggle, hover">
          <FilterToggle title="Show rejected" icon={Eye} isOn={false} onChange={() => undefined} data-hover />
        </State>
        <State name="Toggle, on">
          <FilterToggle title="Starred" icon={Star} isOn onChange={() => undefined} />
        </State>
      </FlowLayout>
      <FlowLayout spacing="small" className="items-end gap-x-large">
        <State name="Search">
          <SearchField
            value={query}
            onChange={setQuery}
            prompt="Search prompts and names"
            className="w-260"
          />
        </State>
        <State name="Search, filled">
          <SearchField
            value="neon"
            onChange={() => undefined}
            prompt="Search prompts and names"
            className="w-260"
          />
        </State>
        <State name="Search, focused">
          <SearchField
            value=""
            onChange={() => undefined}
            prompt="Search prompts and names"
            className="w-260"
            data-focus
          />
        </State>
        <State name="Stepper">
          <ValueStepper label="Variants" value={variants} min={1} max={4} onChange={setVariants} />
        </State>
        <State name="Stepper, at max">
          <ValueStepper label="Variants" value={4} min={1} max={4} onChange={() => undefined} />
        </State>
        <State name="Hint (hover me)">
          <Button variant="icon" icon={Key} title="Hint" hint="Compare two results" />
        </State>
      </FlowLayout>
    </Section>
  );
}

const plate = (
  tier: PlanTier | null,
  badge: string,
  extra: Partial<AccountPlateModel> = {},
): AccountPlateModel => ({
  badge,
  tier,
  amount: "2,596",
  unit: "credits",
  warning: null,
  isAPIKey: false,
  accessibilityLabel: `${badge} plan, 2596 credits left`,
  ...extra,
});

const plates: { name: string; plate: AccountPlateModel; hover?: boolean }[] = [
  { name: "Account, plan unknown", plate: plate(null, "Account", { amount: "—" }) },
  { name: "Free", plate: plate("free", "Free") },
  { name: "Starter", plate: plate("starter", "Basic") },
  { name: "Pro", plate: plate("pro", "Pro") },
  { name: "Premium", plate: plate("premium", "Ultimate") },
  { name: "Elite", plate: plate("elite", "Creator Plus") },
  { name: "Elite, hover", plate: plate("elite", "Creator Plus"), hover: true },
  { name: "Low credits", plate: plate("pro", "Pro", { amount: "42", warning: "Low credits: 42 left" }) },
  {
    name: "API key",
    plate: plate(null, "API key", {
      isAPIKey: true,
      amount: "Per job",
      unit: "billed in USD",
      accessibilityLabel: "API key, billed in USD per job",
    }),
  },
];

function Account() {
  return (
    <Section title="Account plate">
      <FlowLayout spacing="small" className="items-start gap-x-large">
        {plates.map(({ name, plate: model, hover }) => (
          <State key={name} name={name}>
            <div className="w-208">
              <AccountPlate plate={model} onOpen={() => undefined} data-hover={hover} />
            </div>
          </State>
        ))}
      </FlowLayout>
      <FlowLayout>
        <PlanBadge title="Account" tier={null} />
        <PlanBadge title="Free" tier="free" />
        <PlanBadge title="Basic" tier="starter" />
        <PlanBadge title="Pro" tier="pro" />
        <PlanBadge title="Ultimate" tier="premium" />
        <PlanBadge title="Creator Plus" tier="elite" />
        <PlanBadge title="API key" tier={null} icon={Key} />
      </FlowLayout>
    </Section>
  );
}

function Feedback() {
  const { showQueued, showDraftReplaced } = useToastStore();
  return (
    <Section title="Toasts and paged lists">
      <FlowLayout spacing="small" className="items-center gap-x-large">
        <Toast
          icon={CircleCheck}
          text="Queued 2 jobs · 30 credits"
          action={{ title: "View in Queue", run: () => undefined }}
          onDismiss={() => undefined}
        />
        <Button
          variant="secondary"
          title="Show queued toast"
          onClick={() => {
            showQueued("Queued 2 jobs · 30 credits");
          }}
        />
        <Button
          variant="secondary"
          title="Show draft replaced toast"
          onClick={() => {
            showDraftReplaced(() => undefined);
          }}
        />
      </FlowLayout>
      <FlowLayout spacing="small" className="items-start gap-x-large">
        <State name="Feed, failed page">
          <PagedFeedFooter
            failure={{ title: "Couldn't load more" }}
            hasMore
            count={20}
            loadMore={() => undefined}
          />
        </State>
        <State name="Feed, more to load">
          <PagedFeedFooter hasMore count={20} loadMore={() => undefined} />
        </State>
      </FlowLayout>
      <HoverHint text="The app's tooltip, under the control">
        <span className="text-faint">Hover this line for a hint</span>
      </HoverHint>
    </Section>
  );
}
