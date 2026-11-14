import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import {
  commands,
  type AppError,
  events,
  type CreditHistoryView_Serialize,
  type DailyCapView_Serialize,
  type SpendingPeriod,
  type SpendLine,
} from "../../bindings";
import { cx } from "../../design/cx";
import { SegmentedControl } from "../../design/Pickers";
import { Spinner } from "../../design/Spinner";
import { Button, TextField } from "../../design/Styles";
import { useInvalidateOn } from "../../lib/events";
import { useCommand } from "../../lib/ipc";
import { queryKeys } from "../../lib/queryKeys";
import { SettingsRow, SettingsSection, Switch } from "./SettingsRows";
import { useErrorAlert } from "./settingsData";

/**
 * Settings › Spending (SpendingSettingsView.swift): what finished jobs cost per period, the CSV export, the daily cap of
 * the connection and, on an account, the low-credit warning and the credit history.
 */
export function SpendingTab() {
  const [period, setPeriod] = useState<SpendingPeriod>("today");
  const key = queryKeys.settings.spending(period);
  // The last period stays on screen while the next one loads.
  const view = useQuery({
    queryKey: key,
    queryFn: () => commands.spendingView(period),
    placeholderData: keepPreviousData,
  }).data;
  // Today's spend follows every job and charge, and starts again from zero at midnight.
  useInvalidateOn(events.queueChanged, [key]);
  useInvalidateOn(events.libraryChanged, [key]);
  useInvalidateOn(events.imagesChanged, [key]);
  useInvalidateOn(events.connectionChanged, [key, queryKeys.settings.creditHistory]);
  const client = useQueryClient();
  const refreshAfter = view?.refreshAfterMs;
  useEffect(() => {
    if (refreshAfter === undefined) return;
    const timer = window.setTimeout(() => {
      void client.invalidateQueries({ queryKey: key });
    }, refreshAfter);
    return () => {
      window.clearTimeout(timer);
    };
  }, [client, key, refreshAfter]);
  const exportErrors = useErrorAlert("Can't export");
  const changeErrors = useErrorAlert("Can't change this setting");
  if (!view) return null;

  const exportCSV = () => {
    commands.spendingExportCsv().catch(exportErrors.show);
  };

  return (
    <div className="flex flex-col gap-x-large">
      <div className="flex items-center">
        <SegmentedControl value={period} options={view.periods} onChange={setPeriod} />
        <span className="flex-1" />
        <Button variant="secondary" title="Export CSV…" onClick={exportCSV} />
      </div>
      <SettingsSection title="Spent" footer={view.spentFooter}>
        <SettingsRow label="Charged" detail={view.chargedDetail} isFirst>
          <Amount text={view.charged} />
        </SettingsRow>
        <SettingsRow label="Not charged" detail={view.notChargedDetail}>
          <Amount text={view.notCharged} />
        </SettingsRow>
      </SettingsSection>
      {view.breakdowns.map((breakdown) => (
        <Breakdown key={breakdown.title} title={breakdown.title} lines={breakdown.lines} />
      ))}
      <DailyCapSection cap={view.cap} onError={changeErrors.show} />
      {view.lowCredits !== null && <LowCreditSection text={view.lowCredits} onError={changeErrors.show} />}
      {view.showsCreditHistory && <CreditHistorySection />}
      {exportErrors.alert}
      {changeErrors.alert}
    </div>
  );
}

function Amount({ text, isAdded = false }: { text: string; isAdded?: boolean }) {
  return (
    <span
      className={cx(
        "text-body tabular-nums select-text",
        isAdded ? "font-semibold text-accent" : "text-muted",
      )}
    >
      {text}
    </span>
  );
}

function Breakdown({ title, lines }: { title: string; lines: SpendLine[] }) {
  return (
    <SettingsSection title={title}>
      {lines.map((line, index) => (
        <SettingsRow key={line.title} label={line.title} detail={line.jobsText} isFirst={index === 0}>
          <Amount text={line.total} />
        </SettingsRow>
      ))}
    </SettingsSection>
  );
}

/**
 * A field that commits on Return and when it loses focus: Rust keeps the last valid value when the text doesn't parse,
 * and hands back what the field shows next.
 */
export function CommitField({
  label,
  value,
  disabled = false,
  commit,
  onError,
}: {
  label: string;
  value: string;
  disabled?: boolean;
  commit: (text: string) => Promise<string>;
  onError: (error: AppError) => void;
}) {
  const [text, setText] = useState(value);
  const save = () => {
    commit(text).then(setText, onError);
  };
  return (
    <TextField
      aria-label={label}
      value={text}
      disabled={disabled}
      className="w-110 text-right"
      onChange={(event) => {
        setText(event.target.value);
      }}
      onBlur={save}
      onKeyDown={(event) => {
        if (event.key === "Enter") save();
      }}
    />
  );
}

function DailyCapSection({
  cap,
  onError,
}: {
  cap: DailyCapView_Serialize;
  onError: (error: AppError) => void;
}) {
  return (
    <SettingsSection title="Daily cap" footer={cap.footer}>
      <SettingsRow label="Daily cap" isFirst>
        <Switch
          label="Daily cap"
          isOn={cap.isEnabled}
          onChange={(isOn) => {
            commands.spendingSetCapEnabled(isOn).catch(onError);
          }}
        />
      </SettingsRow>
      <SettingsRow label="Amount" isDisabled={!cap.isEnabled}>
        <CommitField
          key={cap.amountText}
          label="Amount"
          value={cap.amountText}
          disabled={!cap.isEnabled}
          commit={(text) => commands.spendingCommitCap(text)}
          onError={onError}
        />
      </SettingsRow>
      <SettingsRow label="Committed today" detail={cap.committedError ?? undefined}>
        <Amount text={cap.committedText} />
      </SettingsRow>
    </SettingsSection>
  );
}

/** When the system tray and the sidebar warn about low credits; accounts only. */
function LowCreditSection({ text, onError }: { text: string; onError: (error: AppError) => void }) {
  return (
    <SettingsSection
      title="Low credits"
      footer="The system tray and the sidebar warn below this many credits."
    >
      <SettingsRow label="Warn below" isFirst>
        <CommitField
          key={text}
          label="Warn below"
          value={text}
          commit={(next) => commands.spendingCommitLowCredits(next)}
          onError={onError}
        />
      </SettingsRow>
    </SettingsSection>
  );
}

/** The account's credit movements as Higgsfield records them, newest first, ten more per Show More. */
function CreditHistorySection() {
  const client = useQueryClient();
  const query = useCommand(queryKeys.settings.creditHistory, () => commands.creditHistory());
  const view = query.data ?? null;
  const [isWorking, setIsWorking] = useState(false);
  const run = (load: () => Promise<CreditHistoryView_Serialize | null>) => {
    setIsWorking(true);
    load()
      .then((next) => {
        client.setQueryData(queryKeys.settings.creditHistory, next);
      })
      .catch(() => undefined)
      .finally(() => {
        setIsWorking(false);
      });
  };
  const isFirst = !view || view.rows.length === 0;

  return (
    <SettingsSection
      title="Credit history"
      footer="From Higgsfield, for every job on the account, including those made on higgsfield.ai."
    >
      {view?.rows.map((row, index) => (
        <SettingsRow
          key={`${row.detail}-${row.title}-${String(index)}`}
          label={row.title}
          detail={row.detail}
          isFirst={index === 0}
        >
          <Amount text={row.amount} isAdded={row.isAdded} />
        </SettingsRow>
      ))}
      {view?.failure ? (
        <SettingsRow label={view.failure.title} detail={view.failure.message} isFirst={isFirst}>
          <Button
            variant="secondary"
            title="Try Again"
            disabled={isWorking}
            onClick={() => {
              run(() => commands.creditHistoryRetry());
            }}
          />
        </SettingsRow>
      ) : query.isPending || isWorking || view?.isLoading === true ? (
        <SettingsRow label="Loading…" isFirst={isFirst}>
          <Spinner />
        </SettingsRow>
      ) : view?.hasOlder ? (
        <SettingsRow label="Older entries" isFirst={isFirst}>
          <Button
            variant="secondary"
            title="Show More"
            onClick={() => {
              run(() => commands.creditHistoryMore());
            }}
          />
        </SettingsRow>
      ) : view?.isEmpty ? (
        <SettingsRow label="No credit movements yet" isFirst />
      ) : null}
    </SettingsSection>
  );
}
