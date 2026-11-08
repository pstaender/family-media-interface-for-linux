import { useCallback, useEffect, useState, type FormEvent } from "react";
import { createRoot } from "react-dom/client";
import {
  MAX_AERIAL_VIDEOS,
  WEEKDAYS,
  websiteDomain,
  type AerialQuality,
  type Config,
  type PongLevel,
  type ScreensaverMode,
  type FolderSettings,
  type Language,
  type LimitRule,
  type Scope,
  type WebsiteSettings,
  type Weekday,
} from "../config";
import { ICONS } from "./icons";

// ---------------------------------------------------------------------------
// API

async function api<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(path, {
    method,
    headers: { "Content-Type": "application/json", "X-Familynovice": "1" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data.error ?? `Request failed (${res.status})`);
  return data as T;
}

type Page = "loading" | "setup" | "login" | "settings";

// ISO 639-2 codes as used on discs, with names for the menu.
const LANGUAGES: [string, string][] = [
  ["deu", "German"],
  ["eng", "English"],
  ["fra", "French"],
  ["spa", "Spanish"],
  ["ita", "Italian"],
  ["nld", "Dutch"],
  ["por", "Portuguese"],
  ["pol", "Polish"],
  ["ces", "Czech"],
  ["dan", "Danish"],
  ["swe", "Swedish"],
  ["nor", "Norwegian"],
  ["fin", "Finnish"],
  ["tur", "Turkish"],
  ["rus", "Russian"],
  ["ell", "Greek"],
  ["hun", "Hungarian"],
  ["ara", "Arabic"],
  ["hin", "Hindi"],
  ["jpn", "Japanese"],
  ["kor", "Korean"],
  ["zho", "Chinese"],
  ["tha", "Thai"],
];
const languageName = (code: string) => LANGUAGES.find(([c]) => c === code)?.[1] ?? code;

// ---------------------------------------------------------------------------
// App

function App() {
  const [page, setPage] = useState<Page>("loading");
  const [setupAllowed, setSetupAllowed] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const s = await api<{ setUp: boolean; loggedIn: boolean; setupAllowed: boolean }>("GET", "/api/state");
      setSetupAllowed(s.setupAllowed);
      setPage(!s.setUp ? "setup" : s.loggedIn ? "settings" : "login");
    } catch (e) {
      setError((e as Error).message);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  if (error) return <Narrow title="familynovice"><div className="banner error">{error}</div></Narrow>;
  if (page === "loading") return null;
  if (page === "setup") {
    if (!setupAllowed) {
      return (
        <Narrow title="Welcome to familynovice">
          <p className="hint">
            The admin password hasn't been set yet. For safety, it can only be set on the media
            center computer itself: open <code className="path">http://127.0.0.1:{location.port || 80}/</code> there.
            Afterwards you can log in from this device.
          </p>
          <button onClick={refresh}>Check again</button>
        </Narrow>
      );
    }
    return <SetupPage onDone={refresh} />;
  }
  if (page === "login") return <LoginPage onDone={refresh} />;
  return <SettingsPage onLogout={refresh} />;
}

function Narrow({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <main>
      <div className="card narrow">
        <h1 style={{ marginBottom: 8 }}>{title}</h1>
        {children}
      </div>
    </main>
  );
}

function SetupPage({ onDone }: { onDone: () => void }) {
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (password !== confirm) return setError("The passwords don't match.");
    setBusy(true);
    try {
      await api("POST", "/api/setup", { password });
      onDone();
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Narrow title="Welcome to familynovice">
      <p className="hint">
        Choose an admin password. You'll need it to change the media center's settings. Children
        shouldn't know it.
      </p>
      {error && <div className="banner error">{error}</div>}
      <form onSubmit={submit}>
        <label className="field">
          <span>Admin password (at least 8 characters)</span>
          <input type="password" autoComplete="new-password" value={password} onChange={(e) => setPassword(e.target.value)} autoFocus />
        </label>
        <label className="field">
          <span>Repeat password</span>
          <input type="password" autoComplete="new-password" value={confirm} onChange={(e) => setConfirm(e.target.value)} />
        </label>
        <button className="primary" disabled={busy || !password}>Set password</button>
      </form>
    </Narrow>
  );
}

function LoginPage({ onDone }: { onDone: () => void }) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    try {
      await api("POST", "/api/login", { password });
      onDone();
    } catch (err) {
      setError((err as Error).message);
      setPassword("");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Narrow title="familynovice admin">
      {error && <div className="banner error">{error}</div>}
      <form onSubmit={submit}>
        <label className="field">
          <span>Admin password</span>
          <input type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} autoFocus />
        </label>
        <button className="primary" disabled={busy || !password}>Log in</button>
      </form>
    </Narrow>
  );
}

// ---------------------------------------------------------------------------
// Layout: left menu, pages with tabs

type Notice = { kind: "ok" | "warn" | "error"; text: string } | null;
type Status = {
  running: boolean;
  /** Media centers of this user besides the one connected to the admin panel. */
  extraInstances?: number;
  extraPids?: number[];
  configError?: string | null;
  error?: string | null;
  locked?: boolean;
  lockReason?: string | null;
  remainingSecs?: number | null;
  usedTodaySecs?: number;
  usedWeekSecs?: number;
  unlockedUntil?: number | null;
  remote?: { devices: string[]; calibrated: boolean; buttons: Record<string, string> } | null;
};

function Icon({ name, size = 20 }: { name: string; size?: number }) {
  const icon = ICONS[name];
  if (!icon) return null;
  // Trusted, generated SVG data (src/web/icons.ts).
  return (
    <svg
      className="icon"
      width={size}
      height={size}
      viewBox={icon.viewBox}
      aria-hidden="true"
      dangerouslySetInnerHTML={{ __html: icon.body }}
    />
  );
}

type TabDef = { id: string; label: string; icon: string; saves?: boolean };
const PAGES: { id: string; label: string; icon: string; tabs: TabDef[] }[] = [
  {
    id: "media",
    label: "Home screen & media",
    icon: "device-tv",
    tabs: [
      { id: "home", label: "Home screen", icon: "home", saves: true },
      { id: "discs", label: "Discs", icon: "disc", saves: true },
      { id: "folders", label: "Folders", icon: "folder", saves: true },
      { id: "websites", label: "Websites", icon: "world", saves: true },
      { id: "screensaver", label: "Screensaver", icon: "moon-stars", saves: true },
      { id: "programs", label: "Programs", icon: "device-gamepad-2", saves: true },
      { id: "remote", label: "Remote", icon: "device-remote" },
    ],
  },
  {
    id: "parental",
    label: "Parental control",
    icon: "shield-lock",
    tabs: [
      { id: "status", label: "Status", icon: "chart-bar" },
      { id: "limits", label: "Time limits", icon: "hourglass", saves: true },
    ],
  },
  {
    id: "security",
    label: "Password & PIN",
    icon: "key",
    tabs: [
      { id: "pin", label: "PIN", icon: "lock", saves: true },
      { id: "password", label: "Admin password", icon: "password" },
    ],
  },
];

/** `#/page/tab` → known page and tab. */
function route(hash: string): { page: (typeof PAGES)[number]; tab: TabDef } {
  const [pageId, tabId] = hash.replace(/^#\/?/, "").split("/");
  const page = PAGES.find((p) => p.id === pageId) ?? PAGES[0]!;
  const tab = page.tabs.find((t) => t.id === tabId) ?? page.tabs[0]!;
  return { page, tab };
}

function SettingsPage({ onLogout }: { onLogout: () => void }) {
  const [config, setConfig] = useState<Config | null>(null);
  const [saved, setSaved] = useState<string>("");
  const [pinSet, setPinSet] = useState(false);
  const [pinAutoSubmit, setPinAutoSubmit] = useState(false);
  const [notice, setNotice] = useState<Notice>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [busy, setBusy] = useState(false);
  const [hash, setHash] = useState(location.hash);

  useEffect(() => {
    const onHash = () => {
      setHash(location.hash);
      setNotice(null);
    };
    addEventListener("hashchange", onHash);
    return () => removeEventListener("hashchange", onHash);
  }, []);

  const loadStatus = useCallback(() => {
    api<Status>("GET", "/api/status").then(setStatus).catch(() => setStatus(null));
  }, []);

  const loadConfig = useCallback(async () => {
    try {
      const r = await api<{ config: Config; pinSet: boolean; pinAutoSubmit: boolean; error?: string }>("GET", "/api/config");
      setConfig(r.config);
      setSaved(JSON.stringify(r.config));
      setPinSet(r.pinSet);
      setPinAutoSubmit(r.pinAutoSubmit);
      if (r.error) setNotice({ kind: "error", text: `${r.error} Saving will replace it.` });
    } catch (e) {
      setNotice({ kind: "error", text: (e as Error).message });
    }
  }, []);

  useEffect(() => {
    loadConfig();
    loadStatus();
    const timer = setInterval(loadStatus, 5000);
    return () => clearInterval(timer);
  }, [loadConfig, loadStatus]);

  if (!config) return <main>{notice && <div className={`banner ${notice.kind}`}>{notice.text}</div>}</main>;

  const { page, tab } = route(hash);
  const dirty = JSON.stringify(config) !== saved;

  const save = async () => {
    setBusy(true);
    setNotice(null);
    try {
      const r = await api<{ saved: boolean; applied: { ok: boolean; error?: string } }>("PUT", "/api/config", config);
      setSaved(JSON.stringify(config));
      if (r.applied.ok) setNotice({ kind: "ok", text: "Saved. The media center uses the new settings now." });
      else setNotice({ kind: "warn", text: `Saved, but not applied yet: ${r.applied.error} The settings apply when the media center starts.` });
      loadStatus();
    } catch (e) {
      setNotice({ kind: "error", text: (e as Error).message });
    } finally {
      setBusy(false);
    }
  };

  const logout = async () => {
    await api("POST", "/api/logout").catch(() => {});
    onLogout();
  };

  let content: React.ReactNode;
  switch (`${page.id}/${tab.id}`) {
    case "media/home":
      content = (
        <>
          <HomeTab config={config} setConfig={setConfig} />
          <RestartCard status={status} reload={loadStatus} setNotice={setNotice} />
        </>
      );
      break;
    case "media/discs":
      content = <DiscsTab config={config} setConfig={setConfig} />;
      break;
    case "media/folders":
      content = <FoldersSection folders={config.folders} onChange={(folders) => setConfig({ ...config, folders })} />;
      break;
    case "media/websites":
      content = <WebsitesSection websites={config.websites} onChange={(websites) => setConfig({ ...config, websites })} />;
      break;
    case "media/programs":
      content = <ProgramsTab config={config} setConfig={setConfig} />;
      break;
    case "media/screensaver":
      content = <ScreensaverTab config={config} setConfig={setConfig} />;
      break;
    case "media/remote":
      content = <RemoteTab status={status} reload={loadStatus} setNotice={setNotice} />;
      break;
    case "parental/status":
      content = <StatusTab status={status} reload={loadStatus} setNotice={setNotice} pinSet={pinSet} />;
      break;
    case "parental/limits":
      content = (
        <LimitsTab
          status={status}
          limits={config.parental.limits}
          onChange={(limits) => setConfig({ ...config, parental: { ...config.parental, limits } })}
        />
      );
      break;
    case "security/pin":
      content = (
        <PinTab
          pinSet={pinSet}
          autoSubmit={pinAutoSubmit}
          doubling={config.parental.pin_backoff_doubling}
          onDoubling={(pin_backoff_doubling) => setConfig({ ...config, parental: { ...config.parental, pin_backoff_doubling } })}
          onPinChanged={async (text) => {
            await loadConfig();
            setNotice({ kind: "ok", text });
          }}
          setNotice={setNotice}
        />
      );
      break;
    default:
      content = <ChangePassword />;
  }

  return (
    <div className="layout">
      <nav className="sidebar">
        <div className="brand">
          <strong>{config.home.title}</strong>
          <span>Admin</span>
        </div>
        <StatusLine status={status} />
        <ul>
          {PAGES.map((p) => (
            <li key={p.id}>
              <a href={`#/${p.id}`} className={p.id === page.id ? "active" : ""}>
                <Icon name={p.icon} /> {p.label}
              </a>
            </li>
          ))}
        </ul>
        <button className="logout" onClick={logout}>
          <Icon name="logout" /> Log out
        </button>
      </nav>

      <main className="content">
        <h1>{page.label}</h1>
        {page.tabs.length > 1 && (
          <div className="tabs" role="tablist">
            {page.tabs.map((t) => (
              <a key={t.id} role="tab" href={`#/${page.id}/${t.id}`} className={t.id === tab.id ? "active" : ""}>
                <Icon name={t.icon} size={18} /> {t.label}
              </a>
            ))}
          </div>
        )}
        {notice && <div className={`banner ${notice.kind}`}>{notice.text}</div>}
        <ExtraInstancesBanner status={status} reload={loadStatus} setNotice={setNotice} />
        {content}
        {tab.saves && (
          <div className="savebar">
            <button className="primary" onClick={save} disabled={busy || !dirty}>
              {busy ? "Saving…" : "Save and apply"}
            </button>
            {dirty && <span className="hint" style={{ margin: 0 }}>Unsaved changes (all tabs)</span>}
          </div>
        )}
      </main>
    </div>
  );
}

function StatusLine({ status }: { status: Status | null }) {
  if (!status) return null;
  if (!status.running) {
    return (
      <span className="status">
        <span className="dot" /> Media center is not running{status.error ? ` (${status.error})` : ""}
      </span>
    );
  }
  return (
    <span className="status">
      <span className={`dot ${status.locked ? "locked" : "on"}`} /> {status.locked ? "Locked" : "Media center is running"}
      {status.configError ? ` — settings problem: ${status.configError}` : ""}
    </span>
  );
}

type ConfigProps = { config: Config; setConfig: (c: Config) => void };

function HomeTab({ config, setConfig }: ConfigProps) {
  return (
    <section className="card">
      <label className="toggle">
        <input
          type="checkbox"
          checked={config.home.show_title}
          onChange={(e) => setConfig({ ...config, home: { ...config.home, show_title: e.target.checked } })}
        />
        <span>
          Show a title
          <small>Shown in the top-left corner of the media center.</small>
        </span>
      </label>
      <label className="field" style={{ marginTop: 6 }}>
        <span>Title</span>
        <input
          type="text"
          maxLength={60}
          value={config.home.title}
          disabled={!config.home.show_title}
          placeholder="familynovice"
          onChange={(e) => setConfig({ ...config, home: { ...config.home, title: e.target.value } })}
          onBlur={() => {
            const title = config.home.title.trim().replace(/\p{Cc}/gu, "") || "familynovice";
            setConfig({ ...config, home: { ...config.home, title } });
          }}
        />
      </label>
      <label className="field" style={{ marginTop: 6 }}>
        <span>Language</span>
        <select
          value={config.home.language}
          onChange={(e) => setConfig({ ...config, home: { ...config.home, language: e.target.value as Language } })}
        >
          <option value="auto">Automatic (the computer's language, otherwise English)</option>
          <option value="en">English</option>
          <option value="de">Deutsch (German)</option>
        </select>
        <small className="hint">Language of the media center on the TV (and of the website window).</small>
      </label>
    </section>
  );
}

function ProgramsTab({ config, setConfig }: ConfigProps) {
  const pong = config.programs.pong;
  const setPong = (patch: Partial<typeof pong>) =>
    setConfig({ ...config, programs: { ...config.programs, pong: { ...pong, ...patch } } });
  return (
    <>
      <section className="card">
        <p className="hint">
          Programs appear in the "Programs" row of the home screen. For now there is a built-in game; programs
          installed on this computer can be added here later.
        </p>
        <label className="toggle">
          <input type="checkbox" checked={pong.enabled} onChange={(e) => setPong({ enabled: e.target.checked })} />
          <span>
            Ping Pong
            <small>
              For 1 player (against the computer) or 2 players against each other. Played with USB gamepads (SNES,
              Xbox or PlayStation style; just plug them in), the remote's arrows or the keyboard (W/S and the arrow
              keys). Counts as usage time like everything else.
            </small>
          </span>
        </label>
        <label className="field" style={{ marginTop: 10 }}>
          <span>The computer plays</span>
          <select
            value={pong.computer_level}
            disabled={!pong.enabled}
            onChange={(e) => setPong({ computer_level: e.target.value as PongLevel })}
          >
            <option value="easy">Easy (for younger children)</option>
            <option value="normal">Normal</option>
            <option value="hard">Hard</option>
          </select>
        </label>
        <label className="field inline">
          <span>Points to win</span>
          <input
            type="number"
            min={3}
            max={21}
            value={pong.points_to_win}
            disabled={!pong.enabled}
            onChange={(e) => setPong({ points_to_win: Math.min(21, Math.max(3, Math.round(Number(e.target.value) || 7))) })}
          />
        </label>
      </section>
    </>
  );
}

type AerialDownload =
  | { status: "done"; bytes: number }
  | { status: "waiting" }
  | { status: "downloading"; received: number; total: number | null }
  | { status: "error"; error: string };
type AerialInfo = {
  videos: { id: string; name: string; location: string; type: string; timeOfDay: string; size: number | null; fallback: boolean }[];
  fetchedAt: number | null;
  error: string | null;
  downloads: Record<string, AerialDownload>;
  freeBytes: number | null;
};

const formatBytes = (bytes: number) =>
  bytes >= 1024 ** 3 ? `${(bytes / 1024 ** 3).toFixed(1)} GB` : `${Math.round(bytes / 1024 ** 2)} MB`;

const VIDEO_GROUPS: [string, string][] = [
  ["landscape", "Landscapes"],
  ["cityscape", "Cities"],
  ["underwater", "Underwater"],
  ["space", "Space"],
  ["other", "Other"],
];

const SCREENSAVER_MODES: { id: ScreensaverMode; label: string; hint: string }[] = [
  { id: "off", label: "Off", hint: "No screensaver. The computer may switch the screen off as usual." },
  {
    id: "title",
    label: "Moving title",
    hint: "The title moves slowly across a very dark screen whose color changes a little every minute. Needs nothing extra.",
  },
  {
    id: "aerial",
    label: "Aerial videos",
    hint: "Flights over landscapes, cities, the sea and Earth from space, like on an Apple TV, in random order without sound. Up to 10 videos are downloaded once and stored on this computer.",
  },
];

function ScreensaverTab({ config, setConfig }: ConfigProps) {
  const s = config.screensaver;
  const set = (patch: Partial<Config["screensaver"]>) => setConfig({ ...config, screensaver: { ...s, ...patch } });
  const [info, setInfo] = useState<AerialInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(
    async (refresh = false) => {
      try {
        setInfo(await api<AerialInfo>("GET", `/api/aerial?quality=${s.aerial_quality}${refresh ? "&refresh=1" : ""}`));
        setError(null);
      } catch (e) {
        setError((e as Error).message);
      }
    },
    [s.aerial_quality],
  );
  useEffect(() => {
    if (s.mode !== "aerial") return;
    load();
    // Sizes arrive in the background, downloads progress.
    const timer = setInterval(() => load(), 4000);
    return () => clearInterval(timer);
  }, [s.mode, load]);

  const chosen = new Set(s.aerial_videos);
  const toggle = (id: string, on: boolean) =>
    set({ aerial_videos: on ? [...s.aerial_videos, id] : s.aerial_videos.filter((v) => v !== id) });
  const total = (info?.videos ?? []).filter((v) => chosen.has(v.id)).reduce((sum, v) => sum + (v.size ?? 0), 0);

  const state = (id: string) => {
    const d = info?.downloads[id];
    if (!d) return <small className="hint">downloads after saving</small>;
    switch (d.status) {
      case "done":
        return <small className="aerial-done">downloaded</small>;
      case "waiting":
        return <small className="hint">waiting</small>;
      case "downloading":
        return <small className="hint">{d.total ? `${Math.floor((d.received / d.total) * 100)} %` : formatBytes(d.received)}</small>;
      case "error":
        return <small className="aerial-error">failed: {d.error}</small>;
    }
  };

  return (
    <>
      <section className="card">
        <p className="hint">
          Shown on the TV after a while without any button press, so no picture stays on the screen for long. Any
          button ends it (and does nothing else). Never shown over a movie or a website; with music it's shown and the
          music keeps playing. While a
          screensaver is set up, the computer doesn't switch the screen off, so the TV keeps its picture.
        </p>
        {SCREENSAVER_MODES.map((m) => (
          <label className="toggle" key={m.id}>
            <input type="radio" name="screensaver-mode" checked={s.mode === m.id} onChange={() => set({ mode: m.id })} />
            <span>
              {m.label}
              <small>{m.hint}</small>
            </span>
          </label>
        ))}
        <label className="field inline" style={{ marginTop: 10 }}>
          <span>Start after</span>
          <input
            type="number"
            min={1}
            max={240}
            value={s.idle_minutes}
            disabled={s.mode === "off"}
            onChange={(e) => set({ idle_minutes: Math.min(240, Math.max(1, Math.round(Number(e.target.value) || 1))) })}
          />
          <small className="hint">minutes without input</small>
        </label>
      </section>

      {s.mode === "aerial" && (
        <section className="card">
          <h2>Aerial videos</h2>
          <label className="field" style={{ marginTop: 10 }}>
            <span>Quality</span>
            <select value={s.aerial_quality} onChange={(e) => set({ aerial_quality: e.target.value as AerialQuality })}>
              <option value="1080p_h264">Full HD, H.264 (works everywhere, 300–600 MB per video)</option>
              <option value="1080p_hevc">Full HD, HEVC (smaller, 100–400 MB; needs a computer from about 2016 on)</option>
              <option value="4k_hevc">4K, HEVC (for 4K TVs, 500–900 MB; needs a recent computer)</option>
            </select>
            <small className="hint">Changing it downloads the chosen videos again; the old files are deleted afterwards.</small>
          </label>
          <p className="hint" style={{ marginBottom: 8 }}>
            <strong>
              {s.aerial_videos.length} of {MAX_AERIAL_VIDEOS} chosen
            </strong>
            {total > 0 && ` · about ${formatBytes(total)}`}
            {info?.freeBytes != null && ` · ${formatBytes(info.freeBytes)} free on this computer`}
            {". "}Downloads start after "Save and apply", one after the other. Until the first one is done, the moving
            title is shown.
          </p>
          {error && <div className="banner error">{error}</div>}
          {info?.error && <div className="banner warn">{info.error}</div>}
          {!info && !error && <p className="hint">Loading the list of videos…</p>}
          {info &&
            VIDEO_GROUPS.map(([type, label]) => {
              const videos = info.videos.filter((v) => v.type === type);
              if (videos.length === 0) return null;
              return (
                <div key={type} className="aerial-group">
                  <h3>{label}</h3>
                  {videos.map((v) => (
                    <label className="aerial-video" key={v.id}>
                      <input
                        type="checkbox"
                        checked={chosen.has(v.id)}
                        disabled={!chosen.has(v.id) && s.aerial_videos.length >= MAX_AERIAL_VIDEOS}
                        onChange={(e) => toggle(v.id, e.target.checked)}
                      />
                      <span className="aerial-name">
                        {v.name}
                        <small>
                          {[v.location, v.timeOfDay].filter(Boolean).join(" · ")}
                          {v.fallback && " · only in 4K"}
                        </small>
                      </span>
                      <span className="aerial-size">
                        {v.size ? formatBytes(v.size) : "…"}
                        {chosen.has(v.id) && state(v.id)}
                      </span>
                    </label>
                  ))}
                </div>
              );
            })}
          {/* Chosen videos that are no longer in the list. */}
          {info &&
            s.aerial_videos
              .filter((id) => !info.videos.some((v) => v.id === id))
              .map((id) => (
                <label className="aerial-video" key={id}>
                  <input type="checkbox" checked onChange={() => toggle(id, false)} />
                  <span className="aerial-name">
                    Unknown video<small>{id} · no longer in the list</small>
                  </span>
                </label>
              ))}
          {info && (
            <p className="hint" style={{ marginTop: 14 }}>
              Videos © Apple, downloaded from Apple's server. List of videos from the{" "}
              <a href="https://github.com/AerialWindows/Aerial" target="_blank" rel="noreferrer">
                AerialWindows/Aerial
              </a>{" "}
              project{info.fetchedAt ? `, updated ${new Date(info.fetchedAt).toLocaleDateString()}` : ""}.{" "}
              <button className="link" onClick={() => load(true)}>
                Update the list
              </button>
            </p>
          )}
        </section>
      )}
    </>
  );
}

/** More than one media center runs: the one on the TV may not get the changes. */
function ExtraInstancesBanner(props: { status: Status | null; reload: () => void; setNotice: (n: Notice) => void }) {
  const [busy, setBusy] = useState(false);
  const extra = props.status?.extraInstances ?? 0;
  if (extra === 0) return null;
  const stop = async () => {
    setBusy(true);
    try {
      const r = await api<{ ended: number }>("POST", "/api/instances/stop-extra");
      props.setNotice({ kind: "ok", text: `Ended ${r.ended === 1 ? "the extra media center" : `${r.ended} extra media centers`}.` });
      setTimeout(props.reload, 1500);
    } catch (e) {
      props.setNotice({ kind: "error", text: (e as Error).message });
    } finally {
      setBusy(false);
    }
  };
  const total = extra + (props.status?.running ? 1 : 0);
  return (
    <div className="banner warn">
      <strong>{total > 1 ? `${total} media centers are running.` : "A media center is running that the admin panel can't reach."}</strong>{" "}
      The one on the TV may not get your changes.{" "}
      {props.status?.running ? (
        <button onClick={stop} disabled={busy}>
          {busy ? "Ending…" : extra === 1 ? "End the extra one" : "End the extra ones"}
        </button>
      ) : (
        "Log the media center account out and in again, or restart the computer."
      )}
      {(props.status?.extraPids?.length ?? 0) > 0 && (
        <small className="banner-hint">
          {props.status?.running ? "If that doesn't help, run" : "Or run"} this in a terminal on the media center
          computer (from an account with admin rights): <code>sudo kill {props.status!.extraPids!.join(" ")}</code>
        </small>
      )}
    </div>
  );
}

function RestartCard(props: { status: Status | null; reload: () => void; setNotice: (n: Notice) => void }) {
  const [busy, setBusy] = useState(false);
  const restart = async () => {
    if (!confirm("Restart the media center? Whatever plays right now stops.")) return;
    setBusy(true);
    try {
      await api("POST", "/api/restart");
      props.setNotice({ kind: "ok", text: "The media center restarts now (the TV is dark for a moment)." });
      setTimeout(props.reload, 4000);
    } catch (e) {
      props.setNotice({ kind: "error", text: (e as Error).message });
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="card">
      <h2>Media center</h2>
      <p className="hint">
        Starts the media center again, e.g. after an update (<code>scripts/install.sh --update</code>) or when it doesn't react.
        Whatever plays right now stops.
      </p>
      <button onClick={restart} disabled={busy || !props.status?.running}>
        <Icon name="power" size={18} /> {busy ? "Restarting…" : "Restart media center"}
      </button>
      {props.status && !props.status.running && <small className="hint">The media center is not running.</small>}
    </section>
  );
}

function DiscsTab({ config, setConfig }: ConfigProps) {
  const setDiscs = (patch: Partial<Config["discs"]>) => setConfig({ ...config, discs: { ...config.discs, ...patch } });
  return (
    <section className="card">
      <p className="hint">DVDs and Blu-rays from the disc drive.</p>
      <label className="toggle">
        <input type="checkbox" checked={config.discs.enabled} onChange={(e) => setDiscs({ enabled: e.target.checked })} />
        <span>
          Allow discs
          <small>When off, the disc tile is shown greyed out and can't be started.</small>
        </span>
      </label>
      <label className="toggle">
        <input
          type="checkbox"
          checked={config.discs.allow_disc_menu}
          disabled={!config.discs.enabled}
          onChange={(e) => setDiscs({ allow_disc_menu: e.target.checked })}
        />
        <span>
          Offer "Disc menu"
          <small>Starts a disc like a normal player, with its own menus, trailers and extras. When off, only the main movie can be played.</small>
        </span>
      </label>
      <LanguageList
        title="Audio language"
        hint="The first language the disc or video has is used."
        value={config.discs.audio_languages}
        onChange={(audio_languages) => setDiscs({ audio_languages })}
        emptyText="Disc default"
      />
      <LanguageList
        title="Subtitles"
        hint="Leave empty for no subtitles."
        value={config.discs.subtitle_languages}
        onChange={(subtitle_languages) => setDiscs({ subtitle_languages })}
        emptyText="No subtitles"
      />
    </section>
  );
}

// ---------------------------------------------------------------------------
// Parental control

const minutesText = (secs: number) => {
  const m = Math.max(0, Math.round(secs / 60));
  return m >= 60 ? `${Math.floor(m / 60)} h ${m % 60} min` : `${m} min`;
};

function RemoteTab(props: { status: Status | null; reload: () => void; setNotice: (n: Notice) => void }) {
  const { status } = props;
  const [busy, setBusy] = useState(false);
  const calibrate = async () => {
    setBusy(true);
    try {
      await api("POST", "/api/remote/calibrate");
      props.setNotice({ kind: "ok", text: "The remote setup is shown on the TV now. Press the buttons it asks for." });
      setTimeout(props.reload, 3000);
    } catch (e) {
      props.setNotice({ kind: "error", text: (e as Error).message });
    } finally {
      setBusy(false);
    }
  };
  if (!status) return <section className="card">Loading…</section>;
  if (!status.running) {
    return (
      <section className="card">
        <p>The media center is not running, so the remote can't be set up right now.</p>
      </section>
    );
  }
  const remote = status.remote;
  return (
    <>
      <section className="card">
        <h2>Remote control</h2>
        <p className="hint">
          Arrow keys, OK as a click and Back as a right click work with most remotes right away. The setup
          learns the other buttons (OK outside pointer mode, home, play/pause, next, previous, volume up / down, mute): the TV asks for
          one button after the other; a button the remote doesn't have is skipped after 10 seconds.
        </p>
        <p className="hint">
          <strong>Switch the remote's mouse / pointer mode off before the setup.</strong> In that mode many
          remotes send different buttons (clicks instead of OK, other media keys), so the setup would learn the
          wrong ones.
        </p>
        {remote && remote.devices.length > 0 ? (
          <p>
            <Icon name="circle-check" /> Read directly: {remote.devices.join(", ")}
          </p>
        ) : (
          <div className="banner warn">
            The media center can't read the remote directly yet, so some buttons (e.g. OK outside pointer mode)
            can't work. On the media center computer, run <code className="path">scripts/remote-access.sh</code>{" "}
            once and choose the remote (it needs an administrator password).
          </div>
        )}
        <button className="primary" disabled={busy} onClick={calibrate}>
          <Icon name="device-remote" /> Set up the remote on the TV
        </button>
      </section>
      {remote?.calibrated && (
        <section className="card">
          <h2>Learned buttons</h2>
          <table className="buttons">
            <tbody>
              {Object.entries(remote.buttons).map(([label, text]) => (
                <tr key={label}>
                  <th>{label}</th>
                  <td>{text}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}
    </>
  );
}

function StatusTab(props: { status: Status | null; reload: () => void; setNotice: (n: Notice) => void; pinSet: boolean }) {
  const { status } = props;
  const [busy, setBusy] = useState(false);
  const act = async (path: string, body?: unknown, text = "Done.") => {
    setBusy(true);
    try {
      await api("POST", path, body);
      props.setNotice({ kind: "ok", text });
      props.reload();
    } catch (e) {
      props.setNotice({ kind: "error", text: (e as Error).message });
    } finally {
      setBusy(false);
    }
  };
  if (!status) return <section className="card">Loading…</section>;
  if (!status.running) {
    return (
      <section className="card">
        <p>The media center is not running, so its state can't be shown or changed right now.</p>
      </section>
    );
  }
  const unlockedUntil = status.unlockedUntil ? new Date(status.unlockedUntil * 1000) : null;
  const unlockActive = !status.locked && unlockedUntil !== null && unlockedUntil > new Date();
  return (
    <>
      <section className={`card state ${status.locked ? "is-locked" : "is-open"}`}>
        <div className="state-head">
          <Icon name={status.locked ? "lock" : "lock-open"} size={32} />
          <div>
            <h2>{status.locked ? "Locked" : "Unlocked"}</h2>
            <p className="hint" style={{ margin: 0 }}>
              {status.locked
                ? status.lockReason
                : status.remainingSecs != null
                  ? `${minutesText(status.remainingSecs)} left`
                  : "No limit right now"}
              {unlockActive
                ? ` · unlocked until ${unlockedUntil.toLocaleString([], { weekday: "short", hour: "2-digit", minute: "2-digit" })}`
                : ""}
            </p>
          </div>
        </div>
        <p style={{ margin: "14px 0 0" }}>
          <Icon name="clock" size={16} /> Used today: <strong>{minutesText(status.usedTodaySecs ?? 0)}</strong>
        </p>
      </section>
      <section className="card">
        <h2>Unlock</h2>
        <p className="hint">Lifts all time limits (and a lock) for this time.</p>
        <div className="row">
          <button disabled={busy} onClick={() => act("/api/unlock", { minutes: 30 }, "Unlocked for 30 minutes.")}>30 minutes</button>
          <button disabled={busy} onClick={() => act("/api/unlock", { minutes: 60 }, "Unlocked for 1 hour.")}>1 hour</button>
          <button disabled={busy} onClick={() => act("/api/unlock", { rest_of_day: true }, "Unlocked for the rest of today.")}>Rest of today</button>
        </div>
      </section>
      {unlockActive && (
        <section className="card">
          <h2>End unlock</h2>
          <p className="hint">
            Removes the remaining unlock time: the time limits apply again right away. If today's time is already
            used up, the media center locks.
          </p>
          <button disabled={busy} onClick={() => act("/api/unlock/end", undefined, "Unlock ended. The time limits apply again.")}>
            <Icon name="hourglass" size={16} /> End unlock
          </button>
        </section>
      )}
      <section className="card">
        <h2>Lock now</h2>
        <p className="hint">
          Stops what's playing and shows the lock screen.{" "}
          {props.pinSet ? "It can be unlocked with the PIN or here." : "Without a PIN it can only be unlocked here."}
        </p>
        <button disabled={busy} onClick={() => act("/api/lock", undefined, "Locked.")}>
          <Icon name="lock" size={16} /> Lock now
        </button>
      </section>
    </>
  );
}

const DAY_NAMES: Record<Weekday, string> = { mon: "Mon", tue: "Tue", wed: "Wed", thu: "Thu", fri: "Fri", sat: "Sat", sun: "Sun" };

const isoDate = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
const todayIso = () => isoDate(new Date());
const mondayIso = () => {
  const d = new Date();
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  return isoDate(d);
};
const formatDate = (iso: string) =>
  new Date(`${iso}T12:00:00`).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" });

/** Expired "today" / "this week" rules don't apply any more. */
function expired(scope: Scope): boolean {
  if (scope.kind === "today") return scope.date < todayIso();
  if (scope.kind === "this_week") return scope.week_of < mondayIso();
  return false;
}

function scopeFor(kind: Scope["kind"]): Scope {
  switch (kind) {
    case "today":
      return { kind, date: todayIso() };
    case "this_week":
      return { kind, week_of: mondayIso() };
    case "weekdays":
      return { kind, days: ["mon", "tue", "wed", "thu", "fri"] };
    default:
      return { kind: "every_day" };
  }
}

/** Does a scope apply to today? */
function appliesToday(scope: Scope): boolean {
  switch (scope.kind) {
    case "today":
      return scope.date === todayIso();
    case "weekdays":
      return scope.days.includes(WEEKDAYS[(new Date().getDay() + 6) % 7]!);
    case "this_week":
      return scope.week_of === mondayIso();
    default:
      return true;
  }
}

function UsageSummary({ status }: { status: Status | null }) {
  if (!status?.running) {
    return (
      <section className="card">
        <p className="hint" style={{ margin: 0 }}>Usage is shown while the media center is running.</p>
      </section>
    );
  }
  return (
    <section className="card usage">
      <div>
        <span className="hint">Used today</span>
        <strong>{minutesText(status.usedTodaySecs ?? 0)}</strong>
      </div>
      <div>
        <span className="hint">Used this week</span>
        <strong>{minutesText(status.usedWeekSecs ?? 0)}</strong>
      </div>
      <div>
        <span className="hint">{status.locked ? "Status" : "Left right now"}</span>
        <strong>
          {status.locked ? "Locked" : status.remainingSecs != null ? minutesText(status.remainingSecs) : "No limit"}
        </strong>
      </div>
    </section>
  );
}

/** "35 of 60 min used" with a bar, for a budget that applies today / this week. */
function BudgetUsage({ rule, status }: { rule: Extract<LimitRule, { type: "budget" }>; status: Status | null }) {
  if (!status?.running || !appliesToday(rule.scope)) return null;
  const week = rule.scope.kind === "this_week";
  const used = (week ? status.usedWeekSecs : status.usedTodaySecs) ?? 0;
  const share = Math.min(1, used / (rule.minutes * 60));
  return (
    <div className="budget-usage">
      <div className="bar">
        <div className={share >= 1 ? "full" : ""} style={{ width: `${share * 100}%` }} />
      </div>
      <span className="hint" style={{ margin: 0 }}>
        {week ? "This week" : "Today"}: {minutesText(used)} of {minutesText(rule.minutes * 60)} used
      </span>
    </div>
  );
}

function LimitsTab({
  limits,
  onChange,
  status,
}: {
  limits: LimitRule[];
  onChange: (l: LimitRule[]) => void;
  status: Status | null;
}) {
  const update = (i: number, rule: LimitRule) => onChange(limits.map((r, j) => (j === i ? rule : r)));
  return (
    <>
      <UsageSummary status={status} />
      <section className="card">
        <p className="hint" style={{ margin: 0 }}>
          When a limit is reached, the media center stops what's playing and locks. It can be unlocked with the
          PIN or on the Status tab. Time only counts while something is used. If several rules apply to a day, the
          most specific wins: <em>today</em> before <em>weekdays</em> before <em>this week</em> before{" "}
          <em>every day</em>. A <em>this week</em> time budget counts the whole week.
        </p>
      </section>
      {limits.length === 0 && (
        <section className="card">
          <p style={{ margin: 0 }}>No limits: the media center can be used any time.</p>
        </section>
      )}
      {limits.map((rule, i) => (
        <LimitEditor
          key={i}
          rule={rule}
          status={status}
          onChange={(r) => update(i, r)}
          onRemove={() => onChange(limits.filter((_, j) => j !== i))}
        />
      ))}
      <div className="row" style={{ marginBottom: 16 }}>
        <button onClick={() => onChange([...limits, { type: "budget", minutes: 60, scope: { kind: "every_day" } }])}>
          <Icon name="hourglass" size={16} /> Add time budget
        </button>
        <button onClick={() => onChange([...limits, { type: "window", from: "14:00", until: "19:00", scope: { kind: "every_day" } }])}>
          <Icon name="clock" size={16} /> Add time window
        </button>
      </div>
    </>
  );
}

function LimitEditor(props: {
  rule: LimitRule;
  status: Status | null;
  onChange: (r: LimitRule) => void;
  onRemove: () => void;
}) {
  const { rule, onChange } = props;
  const scope = rule.scope;
  const setScope = (s: Scope) => onChange({ ...rule, scope: s });
  const old = expired(scope);
  return (
    <section className={`card limit ${old ? "expired" : ""}`}>
      <div className="row" style={{ justifyContent: "space-between" }}>
        <strong>
          <Icon name={rule.type === "budget" ? "hourglass" : "clock"} size={18} />{" "}
          {rule.type === "budget" ? "Time budget" : "Time window"}
          {old && <span className="badge">expired</span>}
        </strong>
        <button className="small" onClick={props.onRemove} aria-label="Remove rule">
          <Icon name="trash" size={16} /> Remove
        </button>
      </div>

      {rule.type === "budget" ? (
        <label className="field inline">
          <span>Minutes</span>
          <input
            type="number"
            min={1}
            max={scope.kind === "this_week" ? 10080 : 1440}
            value={rule.minutes}
            onChange={(e) => onChange({ ...rule, minutes: Math.max(1, Math.round(Number(e.target.value) || 1)) })}
          />
          <small className="hint">{scope.kind === "this_week" ? "for the whole week" : "per day"}</small>
        </label>
      ) : (
        <div className="row">
          <label className="check">
            <input
              type="checkbox"
              checked={rule.from !== undefined}
              onChange={(e) => {
                const { from: _from, ...rest } = rule;
                onChange(e.target.checked ? { ...rest, from: "14:00" } : rest);
              }}
            />{" "}
            From
          </label>
          <input
            type="time"
            value={rule.from ?? ""}
            disabled={rule.from === undefined}
            onChange={(e) => e.target.value && onChange({ ...rule, from: e.target.value })}
          />
          <span>until</span>
          <input type="time" value={rule.until} onChange={(e) => e.target.value && onChange({ ...rule, until: e.target.value })} />
        </div>
      )}

      <div className="row" style={{ marginTop: 10 }}>
        <span>Applies</span>
        <select value={scope.kind} onChange={(e) => setScope(scopeFor(e.target.value as Scope["kind"]))}>
          <option value="today">today</option>
          <option value="weekdays">on specific weekdays</option>
          <option value="this_week">this week</option>
          <option value="every_day">every day</option>
        </select>
        {scope.kind === "today" && <span className="hint" style={{ margin: 0 }}>({formatDate(scope.date)})</span>}
        {scope.kind === "this_week" && (
          <span className="hint" style={{ margin: 0 }}>(week from {formatDate(scope.week_of)})</span>
        )}
        {old && (
          <button className="small" onClick={() => setScope(scopeFor(scope.kind))}>
            Use for {scope.kind === "today" ? "today" : "this week"}
          </button>
        )}
      </div>
      {rule.type === "budget" && <BudgetUsage rule={rule} status={props.status} />}
      {scope.kind === "weekdays" && (
        <div className="row days">
          {WEEKDAYS.map((d) => (
            <label key={d} className={`day ${scope.days.includes(d) ? "on" : ""}`}>
              <input
                type="checkbox"
                checked={scope.days.includes(d)}
                onChange={(e) => {
                  const days = e.target.checked
                    ? WEEKDAYS.filter((x) => x === d || scope.days.includes(x))
                    : scope.days.filter((x) => x !== d);
                  setScope({ kind: "weekdays", days });
                }}
              />
              {DAY_NAMES[d]}
            </label>
          ))}
        </div>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// PIN

function PinTab(props: {
  pinSet: boolean;
  autoSubmit: boolean;
  doubling: boolean;
  onDoubling: (v: boolean) => void;
  onPinChanged: (text: string) => Promise<void>;
  setNotice: (n: Notice) => void;
}) {
  const [pin, setPin] = useState("");
  const [repeat, setRepeat] = useState("");
  const [busy, setBusy] = useState(false);
  const valid = /^\d{4,8}$/.test(pin);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (pin !== repeat) return props.setNotice({ kind: "error", text: "The PINs don't match." });
    setBusy(true);
    try {
      await api("POST", "/api/pin", { pin });
      setPin("");
      setRepeat("");
      await props.onPinChanged(props.pinSet ? "PIN changed." : "PIN set.");
    } catch (err) {
      props.setNotice({ kind: "error", text: (err as Error).message });
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    setBusy(true);
    try {
      await api("DELETE", "/api/pin");
      await props.onPinChanged("PIN removed.");
    } catch (err) {
      props.setNotice({ kind: "error", text: (err as Error).message });
    } finally {
      setBusy(false);
    }
  };

  const digits = (v: string) => v.replace(/\D/g, "").slice(0, 8);
  return (
    <>
      <section className="card">
        <h2>{props.pinSet ? "Change the PIN" : "Set a PIN"}</h2>
        <p className="hint">
          {props.pinSet ? (
            <>
              <Icon name="circle-check" size={16} /> A PIN is set. It unlocks the media center when the time is up, and
              enables the lock button on the home screen.
            </>
          ) : (
            "With a PIN, the media center can be unlocked on the TV when the time is up, and the lock button appears on the home screen."
          )}
        </p>
        {props.pinSet && !props.autoSubmit && (
          <div className="banner warn">
            This PIN was set before its number of digits was stored, so the media center needs OK after entering it.
            Set the PIN once more (the same digits are fine) and it is checked as soon as the last digit is entered.
          </div>
        )}
        <form onSubmit={submit}>
          <div className="row">
            <label className="field" style={{ flex: 1 }}>
              <span>New PIN (4 to 8 digits)</span>
              <input type="password" inputMode="numeric" autoComplete="new-password" value={pin} onChange={(e) => setPin(digits(e.target.value))} />
            </label>
            <label className="field" style={{ flex: 1 }}>
              <span>Repeat PIN</span>
              <input type="password" inputMode="numeric" autoComplete="new-password" value={repeat} onChange={(e) => setRepeat(digits(e.target.value))} />
            </label>
          </div>
          <div className="row">
            <button className="primary" disabled={busy || !valid || !repeat}>{props.pinSet ? "Change PIN" : "Set PIN"}</button>
            {props.pinSet && (
              <button type="button" disabled={busy} onClick={remove}>
                <Icon name="trash" size={16} /> Remove PIN
              </button>
            )}
          </div>
        </form>
      </section>
      <section className="card">
        <h2>Wrong PIN</h2>
        <label className="toggle">
          <input type="checkbox" checked={props.doubling} onChange={(e) => props.onDoubling(e.target.checked)} />
          <span>
            Double the waiting time after each wrong PIN
            <small>
              After a wrong PIN the next try is possible after 5 seconds. With this option the waiting time doubles
              each time: 5 s, 10 s, 20 s, 40 s … (at most 1 hour).
            </small>
          </span>
        </label>
      </section>
    </>
  );
}

function LanguageList(props: {
  title: string;
  hint: string;
  value: string[];
  emptyText: string;
  onChange: (value: string[]) => void;
}) {
  const { value, onChange } = props;
  const available = LANGUAGES.filter(([code]) => !value.includes(code));
  const [pick, setPick] = useState("");

  const move = (i: number, delta: number) => {
    const next = [...value];
    const j = i + delta;
    [next[i], next[j]] = [next[j]!, next[i]!];
    onChange(next);
  };

  return (
    <div className="languages">
      <strong>{props.title}</strong>
      <div className="hint" style={{ marginBottom: 0 }}>{props.hint}</div>
      {value.length === 0 ? (
        <p style={{ margin: "8px 0" }}>{props.emptyText}</p>
      ) : (
        <ol>
          {value.map((code, i) => (
            <li key={code}>
              {languageName(code)}
              <span className="actions">
                <button className="small" disabled={i === 0} onClick={() => move(i, -1)} aria-label="Move up">↑</button>
                <button className="small" disabled={i === value.length - 1} onClick={() => move(i, 1)} aria-label="Move down">↓</button>
                <button className="small" onClick={() => onChange(value.filter((c) => c !== code))} aria-label="Remove">✕</button>
              </span>
            </li>
          ))}
        </ol>
      )}
      <div className="add-language">
        <select value={pick} onChange={(e) => setPick(e.target.value)}>
          <option value="">Add a language…</option>
          {available.map(([code, name]) => (
            <option key={code} value={code}>{name}</option>
          ))}
        </select>
        <button
          disabled={!pick}
          onClick={() => {
            onChange([...value, pick]);
            setPick("");
          }}
        >
          Add
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Folders

function newFolderId(name: string, taken: string[]): string {
  const slug = name.toLowerCase().normalize("NFKD").replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 30) || "folder";
  let id = slug;
  while (taken.includes(id)) id = `${slug}-${Math.random().toString(36).slice(2, 6)}`;
  return id;
}

function FoldersSection(props: { folders: FolderSettings[]; onChange: (folders: FolderSettings[]) => void }) {
  const { folders, onChange } = props;
  const [picking, setPicking] = useState<number | "new" | null>(null);

  const update = (i: number, patch: Partial<FolderSettings>) =>
    onChange(folders.map((f, j) => (j === i ? { ...f, ...patch } : f)));

  const picked = (path: string) => {
    if (picking === "new") {
      const base = path === "/" ? "Computer" : path.split("/").pop()!;
      const name = base.slice(0, 60);
      onChange([
        ...folders,
        { id: newFolderId(name, folders.map((f) => f.id)), name, path, video: true, audio: true, images: true },
      ]);
    } else if (typeof picking === "number") {
      update(picking, { path });
    }
    setPicking(null);
  };

  return (
    <section className="card">
      <p className="hint">
        Folders appear as tiles on the media center and can be browsed there. Only videos, music and
        pictures are shown. familynovice only reads them — it never changes, moves or deletes files.
      </p>
      {folders.length === 0 && <p>No folders yet.</p>}
      {folders.map((folder, i) => (
        <FolderEditor
          key={folder.id}
          folder={folder}
          onChange={(patch) => update(i, patch)}
          onRemove={() => onChange(folders.filter((_, j) => j !== i))}
          onPick={() => setPicking(i)}
        />
      ))}
      {picking !== null ? (
        <FolderPicker
          start={typeof picking === "number" ? folders[picking]?.path : undefined}
          onPick={picked}
          onCancel={() => setPicking(null)}
        />
      ) : (
        folders.length < 50 && <button onClick={() => setPicking("new")}>Add folder…</button>
      )}
    </section>
  );
}

function FolderEditor(props: {
  folder: FolderSettings;
  onChange: (patch: Partial<FolderSettings>) => void;
  onRemove: () => void;
  onPick: () => void;
}) {
  const { folder, onChange } = props;
  const nothing = !folder.video && !folder.audio && !folder.images;

  return (
    <div className="folder">
      <div className="row" style={{ justifyContent: "space-between" }}>
        <label className="field" style={{ flex: 1, marginBottom: 8 }}>
          <span>Name on the tile</span>
          <input
            type="text"
            maxLength={60}
            value={folder.name}
            onChange={(e) => onChange({ name: e.target.value })}
            onBlur={() => onChange({ name: folder.name.trim().replace(/\p{Cc}/gu, "") })}
          />
        </label>
        <button onClick={props.onRemove} aria-label={`Remove ${folder.name}`}>Remove</button>
      </div>
      <div className="row" style={{ marginBottom: 8 }}>
        <code className="path">{folder.path}</code>
        <button className="small" onClick={props.onPick}>Change…</button>
      </div>
      <div className="row kinds">
        {(
          [
            ["video", "Videos"],
            ["audio", "Music"],
            ["images", "Pictures"],
          ] as const
        ).map(([key, label]) => (
          <label key={key} className="check">
            <input type="checkbox" checked={folder[key]} onChange={(e) => onChange({ [key]: e.target.checked })} /> {label}
          </label>
        ))}
      </div>
      {nothing && <div className="banner warn">Choose at least one kind of file to show.</div>}
    </div>
  );
}

function FolderPicker(props: { start?: string; onPick: (path: string) => void; onCancel: () => void }) {
  const [listing, setListing] = useState<{ path: string; parent: string | null; dirs: string[]; error?: string } | null>(null);

  const open = useCallback(async (path?: string) => {
    const query = path ? `?path=${encodeURIComponent(path)}` : "";
    const res = await fetch(`/api/browse${query}`);
    const data = await res.json();
    if (res.ok) setListing(data);
    else setListing({ path: data.path ?? path ?? "/", parent: data.parent ?? null, dirs: [], error: data.error });
  }, []);

  useEffect(() => {
    open(props.start);
  }, [open, props.start]);

  if (!listing) return null;
  const child = (name: string) => (listing.path === "/" ? `/${name}` : `${listing.path}/${name}`);

  return (
    <div className="picker">
      <div className="row" style={{ justifyContent: "space-between" }}>
        <code className="path">{listing.path}</code>
        <button className="small" disabled={listing.parent === null} onClick={() => open(listing.parent ?? "/")}>↑ Up</button>
      </div>
      {listing.error && <div className="banner error" style={{ marginTop: 8 }}>{listing.error}</div>}
      <ul className="dirs">
        {listing.dirs.length === 0 && !listing.error && <li className="hint">No subfolders</li>}
        {listing.dirs.map((d) => (
          <li key={d}>
            <button className="link" onClick={() => open(child(d))}>📁 {d}</button>
          </li>
        ))}
      </ul>
      <div className="row">
        <button className="primary" disabled={!!listing.error} onClick={() => props.onPick(listing.path)}>
          Use this folder
        </button>
        <button onClick={props.onCancel}>Cancel</button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Websites

function WebsitesSection(props: { websites: WebsiteSettings[]; onChange: (websites: WebsiteSettings[]) => void }) {
  const { websites, onChange } = props;
  const update = (i: number, patch: Partial<WebsiteSettings>) =>
    onChange(websites.map((w, j) => (j === i ? { ...w, ...patch } : w)));

  const add = () => {
    const taken = websites.map((w) => w.id);
    let id = "website";
    for (let n = 2; taken.includes(id); n++) id = `website-${n}`;
    onChange([...websites, { id, name: "", url: "https://", allow_subdomains: true, show_logo: true }]);
  };

  return (
    <section className="card">
      <p className="hint">
        Websites open full screen without an address bar. Children can only open pages of the website's
        domain; pictures, videos and players from other addresses (e.g. CDNs) still load. Closing: the ✕
        button, or holding Esc for 3 seconds.
      </p>
      {websites.length === 0 && <p>No websites yet.</p>}
      {websites.map((site, i) => (
        <WebsiteEditor
          key={site.id}
          site={site}
          onChange={(patch) => update(i, patch)}
          onRemove={() => onChange(websites.filter((_, j) => j !== i))}
        />
      ))}
      {websites.length < 50 && <button onClick={add}>Add website</button>}
    </section>
  );
}

function WebsiteEditor(props: {
  site: WebsiteSettings;
  onChange: (patch: Partial<WebsiteSettings>) => void;
  onRemove: () => void;
}) {
  const { site, onChange } = props;
  const domain = site.url === "https://" ? null : websiteDomain(site.url, site.allow_subdomains);

  return (
    <div className="folder">
      <div className="row" style={{ justifyContent: "space-between" }}>
        <label className="field" style={{ flex: 1, marginBottom: 8 }}>
          <span>Name on the tile</span>
          <input
            type="text"
            maxLength={60}
            value={site.name}
            placeholder="e.g. KiKA"
            onChange={(e) => onChange({ name: e.target.value })}
            onBlur={() => onChange({ name: site.name.trim().replace(/\p{Cc}/gu, "") })}
          />
        </label>
        <button onClick={props.onRemove} aria-label={`Remove ${site.name || "website"}`}>Remove</button>
      </div>
      <label className="field">
        <span>Start page</span>
        <input
          type="text"
          inputMode="url"
          value={site.url}
          placeholder="https://www.example.com/"
          onChange={(e) => onChange({ url: e.target.value.trim() })}
        />
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={site.allow_subdomains}
          onChange={(e) => onChange({ allow_subdomains: e.target.checked })}
        />{" "}
        Allow all subdomains
      </label>{" "}
      <label className="check" style={{ marginLeft: 18 }}>
        <input type="checkbox" checked={site.show_logo} onChange={(e) => onChange({ show_logo: e.target.checked })} />{" "}
        Show the website's logo on the tile
      </label>
      {site.show_logo && <LogoPreview id={site.id} url={site.url} />}
      {domain !== null && (
        <p className="hint" style={{ margin: "8px 0 0" }}>
          {typeof domain === "string" ? (
            <span style={{ color: "var(--err-text)" }}>{domain}</span>
          ) : domain.subdomains ? (
            <>
              Pages allowed on <strong>{domain.host}</strong> and all its subdomains (e.g. www.{domain.host}).
            </>
          ) : (
            <>
              Pages allowed only on <strong>{domain.host}</strong>.
            </>
          )}
        </p>
      )}
    </div>
  );
}

/**
 * The logo stored for a website. It is looked up when the settings are saved (in the
 * background), so right after saving there may be none yet; "Look again" fetches it now.
 */
function LogoPreview({ id, url }: { id: string; url: string }) {
  const [version, setVersion] = useState(0);
  const [state, setState] = useState<"loading" | "shown" | "none">("loading");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Saving looks for the logo in the background: check again a few times.
  useEffect(() => {
    if (state !== "none") return;
    const timer = setTimeout(() => setVersion((v) => v + 1), 4000);
    return () => clearTimeout(timer);
  }, [state, version]);

  const lookAgain = async () => {
    setBusy(true);
    setError(null);
    try {
      await api("POST", `/api/websites/${id}/logo`);
      setVersion((v) => v + 1);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="logo-preview">
      <div className="logo-plate">
        <img
          key={`${version}-${url}`}
          src={`/api/websites/${id}/logo?v=${version}`}
          alt=""
          style={{ display: state === "shown" ? "block" : "none" }}
          onLoad={() => setState("shown")}
          onError={() => setState("none")}
        />
        {state !== "shown" && <span className="hint">no logo yet</span>}
      </div>
      <div>
        <p className="hint" style={{ margin: 0 }}>
          {state === "shown"
            ? "This logo is shown on the tile."
            : "The logo is looked up on the website after saving. Without one, the tile shows a globe."}
        </p>
        <button className="small" disabled={busy} onClick={lookAgain}>
          {busy ? "Looking…" : "Look again"}
        </button>
        {error && <p className="hint" style={{ color: "var(--err-text)", margin: "4px 0 0" }}>{error}</p>}
      </div>
    </div>
  );
}

function ChangePassword() {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [notice, setNotice] = useState<Notice>(null);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    try {
      await api("POST", "/api/password", { current, next });
      setNotice({ kind: "ok", text: "Password changed." });
      setCurrent("");
      setNext("");
    } catch (err) {
      setNotice({ kind: "error", text: (err as Error).message });
    }
  };

  return (
    <section className="card">
      <form onSubmit={submit}>
        {notice && <div className={`banner ${notice.kind}`}>{notice.text}</div>}
        <label className="field">
          <span>Current password</span>
          <input type="password" autoComplete="current-password" value={current} onChange={(e) => setCurrent(e.target.value)} />
        </label>
        <label className="field">
          <span>New password (at least 8 characters)</span>
          <input type="password" autoComplete="new-password" value={next} onChange={(e) => setNext(e.target.value)} />
        </label>
        <button className="primary" disabled={!current || !next}>Change password</button>
      </form>
    </section>
  );
}

createRoot(document.getElementById("root")!).render(<App />);
