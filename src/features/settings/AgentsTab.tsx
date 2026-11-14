import { commands, type AgentClientSetup } from "../../bindings";
import { CopyButton } from "../../design/Styles";
import { SettingsRow, SettingsSection, Switch } from "./SettingsRows";
import { useAgentSettings, useErrorAlert } from "./settingsData";
import { CommitField } from "./SpendingTab";

/**
 * Settings › Agents (AgentSettingsView.swift): Allow agents and whether the app listens, the daily agent budget of each
 * connection, the unlimited allowance, and how to connect an agent to the bundled `jutsu` helper. Every value comes
 * from Rust; `PreferencesChanged` brings the tab up to date.
 */
export function AgentsTab() {
  const settings = useAgentSettings().data;
  const errors = useErrorAlert("Can't change this setting");
  if (!settings) return null;

  return (
    <div className="flex flex-col gap-x-large">
      <SettingsSection
        title="Agents"
        footer="Lets an AI agent on this computer, such as Claude Code, make videos within the budget below, follow their jobs and open the app's pages. Agents never see your API key or sign-in and can't delete anything. Turning this off refuses their next request and stops nothing already running."
      >
        <SettingsRow label="Allow agents" detail={settings.status} isFirst>
          <Switch
            label="Allow agents"
            isOn={settings.isAllowed}
            onChange={(isOn) => {
              commands.agentsSetAllowed(isOn).catch(errors.show);
            }}
          />
        </SettingsRow>
      </SettingsSection>
      <SettingsSection
        title="Daily agent budget"
        footer="What agents may spend a day; your daily cap still applies. 0 lets them look but not spend. A job over its limits waits in the Queue for you to confirm."
      >
        <SettingsRow label="With an API key (USD)" isFirst>
          <CommitField
            key={settings.centsText}
            label="With an API key (USD)"
            value={settings.centsText}
            commit={(text) => commands.agentsCommitBudget("apiKey", text)}
            onError={errors.show}
          />
        </SettingsRow>
        <SettingsRow label="With an account (credits)">
          <CommitField
            key={settings.creditsText}
            label="With an account (credits)"
            value={settings.creditsText}
            commit={(text) => commands.agentsCommitBudget("account", text)}
            onError={errors.show}
          />
        </SettingsRow>
      </SettingsSection>
      <SettingsSection
        title="Unlimited generations"
        footer="Off, an agent's job that your plan's unlimited generations would pay for waits in the Queue for you to confirm. On, it runs without counting against the agent budget."
      >
        <SettingsRow label="Let agents use unlimited generations" isFirst>
          <Switch
            label="Let agents use unlimited generations"
            isOn={settings.allowsUnlimited}
            onChange={(isOn) => {
              commands.agentsSetAllowsUnlimited(isOn).catch(errors.show);
            }}
          />
        </SettingsRow>
      </SettingsSection>
      {settings.helperPath === null ? (
        <SettingsSection title="Connect your agent">
          <SettingsRow label="The jutsu helper isn't in this build" isFirst />
        </SettingsSection>
      ) : (
        settings.clients.map((client) => <ClientSection key={client.name} client={client} />)
      )}
      {errors.alert}
    </div>
  );
}

/** Claude Code's setup (`AgentSettingsView.swift`): what to do and the command to copy. */
function ClientSection({ client }: { client: AgentClientSetup }) {
  return (
    <SettingsSection title={client.name} footer={client.instructions}>
      <SettingsRow label="Command" isFirst>
        <CopyButton text={client.snippet} />
      </SettingsRow>
      <pre className="m-0 overflow-x-auto pb-large font-mono text-[12px] whitespace-pre-wrap text-muted select-text">
        {client.snippet}
      </pre>
    </SettingsSection>
  );
}
