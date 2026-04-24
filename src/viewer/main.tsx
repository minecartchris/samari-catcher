import { Styles } from "@squid-dev/cc-web-term";
import { Component, JSX, h } from "preact";
import { checkToken, genToken } from "../token";
import type { Token } from "../token";
import { windowTitle } from "./computer";
import { Cog } from "./font";
import { Session, SessionInfo } from "./session";
import { Settings } from "./settings";
import {
  container, dialogueOverlay, sessionHidden, sessionHost, settingsCog,
  tabActive, tabBar, tabClose, tabEntry, tabLabel, tabNewButton, tokenButton,
} from "./styles.css";
import termFont from "@squid-dev/cc-web-term/assets/term_font.png";

const TABS_KEY = "tabs";
const ACTIVE_KEY = "activeTab";

export type MainProps = Record<string, never>;

type TabInfo = {
  token: Token,
  id: number | null,
  label: string | null,
};

type MainState = {
  tabs: TabInfo[],
  activeToken: Token,
  settings: Settings,
  dialogue?: (state: MainState) => JSX.Element,
};

const loadSavedTokens = (): Token[] => {
  try {
    const raw = window.localStorage.getItem(TABS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(checkToken);
  } catch {
    return [];
  }
};

const loadActiveToken = (): Token | null => {
  try {
    const raw = window.localStorage.getItem(ACTIVE_KEY);
    return checkToken(raw) ? raw : null;
  } catch {
    return null;
  }
};

const getUrlToken = (): Token | null => {
  const queryArgs = window.location.search
    .substring(1).split("&")
    .map(x => x.split("=", 2).map(decodeURIComponent));
  for (const [k, v] of queryArgs) {
    if (k === "id" && checkToken(v)) return v;
  }
  return null;
};

const tabDisplayName = (tab: TabInfo): string => {
  if (tab.label) return tab.label;
  if (tab.id !== null) return `Computer #${tab.id}`;
  return tab.token.substring(0, 8);
};

export class Main extends Component<MainProps, MainState> {
  public constructor(props: MainProps, context: any) {
    super(props, context);

    const saved = loadSavedTokens();
    const urlToken = getUrlToken();
    const persistedActive = loadActiveToken();

    const tokens: Token[] = [...saved];
    if (urlToken && !tokens.includes(urlToken)) tokens.push(urlToken);
    if (tokens.length === 0) tokens.push(genToken());

    const tabs: TabInfo[] = tokens.map(token => ({ token, id: null, label: null }));

    const activeToken = urlToken && tokens.includes(urlToken)
      ? urlToken
      : (persistedActive && tokens.includes(persistedActive) ? persistedActive : tokens[0]);

    const settings: Settings = {
      showInvisible: true,
      trimWhitespace: true,

      terminalFont: termFont,

      darkMode: false,
      terminalBorder: false,
    };

    try {
      const settingJson = window.localStorage.settings;
      if (settingJson !== undefined) {
        const settingStorage = JSON.parse(settingJson);
        for (const key of Object.keys(settings)) {
          const value = settingStorage[key];
          if (value !== undefined) (settings as any)[key] = value;
        }
      }
    } catch {
      // Ignore
    }

    this.state = {
      tabs,
      activeToken,
      settings,
    };
  }

  public override componentDidMount() {
    this.persistTabs();
    this.syncUrl();
    this.syncDocument();
  }

  public override componentDidUpdate() {
    try {
      window.localStorage.settings = JSON.stringify(this.state.settings);
    } catch {
      // Ignore
    }

    this.persistTabs();
    this.syncUrl();
    this.syncDocument();
  }

  private persistTabs() {
    try {
      window.localStorage.setItem(TABS_KEY, JSON.stringify(this.state.tabs.map(t => t.token)));
      window.localStorage.setItem(ACTIVE_KEY, this.state.activeToken);
    } catch {
      // Ignore
    }
  }

  private syncUrl() {
    const target = `${window.location.origin}${window.location.pathname}?id=${this.state.activeToken}`;
    if (target !== window.location.href && window.history.replaceState) {
      window.history.replaceState({ id: this.state.activeToken }, window.name, target);
    }
  }

  private syncDocument() {
    const active = this.state.tabs.find(t => t.token === this.state.activeToken);
    document.body.setAttribute("data-theme", this.state.settings.darkMode ? "dark" : "light");
    document.title = windowTitle(active?.id ?? null, active?.label ?? null);
  }

  public override render(_props: MainProps, state: MainState) {
    return <div class={container}>
      <div class={tabBar}>
        {state.tabs.map(tab => {
          const isActive = tab.token === state.activeToken;
          return <div key={tab.token} class={`${tabEntry} ${isActive ? tabActive : ""}`}
            title={tab.token}
            onClick={this.selectTab(tab.token)}>
            <div class={tabLabel}>{tabDisplayName(tab)}</div>
            <div class={tabClose} title="Close tab"
              onClick={this.closeTab(tab.token)}></div>
          </div>;
        })}
        <div class={tabNewButton} title="New computer tab" onClick={this.addTab}>+</div>
        <div class={tokenButton} title="Add token" onClick={this.addToken}>+token</div>
      </div>
      <div class={sessionHost}>
        {state.tabs.map(tab => {
          const isActive = tab.token === state.activeToken;
          return <div key={tab.token} class={isActive ? "" : sessionHidden}>
            <Session token={tab.token}
              settings={state.settings}
              focused={isActive && state.dialogue === undefined}
              onInfo={this.onSessionInfo} />
          </div>;
        })}
      </div>
      <button class={`${Styles.actionButton} ${settingsCog}`}
        title="Configure how CloudCatcher behaves"
        onClick={this.openSettings}>
        <Cog />
      </button>
      {
        state.dialogue ?
          <div class={dialogueOverlay} onClick={this.closeDialogueClick}>
            {state.dialogue(state)}
          </div> : ""
      }
    </div>;
  }

  private onSessionInfo = (token: Token, info: SessionInfo) => {
    this.setState({
      tabs: this.state.tabs.map(t => t.token === token ? { ...t, ...info } : t),
    });
  }

  private selectTab = (token: Token) => (e: Event) => {
    e.stopPropagation();
    if (token !== this.state.activeToken) this.setState({ activeToken: token });
  }

  private closeTab = (token: Token) => (e: Event) => {
    e.stopPropagation();

    const remaining = this.state.tabs.filter(t => t.token !== token);
    if (remaining.length === 0) {
      const fresh: TabInfo = { token: genToken(), id: null, label: null };
      this.setState({ tabs: [fresh], activeToken: fresh.token });
      return;
    }

    let active = this.state.activeToken;
    if (active === token) {
      const closedIndex = this.state.tabs.findIndex(t => t.token === token);
      const next = remaining[Math.min(closedIndex, remaining.length - 1)];
      active = next.token;
    }
    this.setState({ tabs: remaining, activeToken: active });
  }

  private addTab = (e: Event) => {
    e.stopPropagation();
    const token = genToken();
    this.setState({
      tabs: [...this.state.tabs, { token, id: null, label: null }],
      activeToken: token,
    });
  }

  private addToken = (e: Event) => {
    e.stopPropagation();
    const token = prompt("Enter token:");
    if (token && checkToken(token)) {
      this.setState({
        tabs: [...this.state.tabs, { token, id: null, label: null }],
        activeToken: token,
      });
    }
  }

  private openSettings = () => {
    const update = (s: Settings) => this.setState({ settings: s });
    this.setState({ dialogue: (s: MainState) => <Settings settings={s.settings} update={update} /> });
  }

  private closeDialogueClick = (e: MouseEvent) => {
    if (e.target === e.currentTarget) this.setState({ dialogue: undefined });
  }
}
