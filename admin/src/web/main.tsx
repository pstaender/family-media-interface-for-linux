import { useCallback, useEffect, useState, type FormEvent } from "react";
import { createRoot } from "react-dom/client";
import type { Config } from "../config";

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
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const s = await api<{ setUp: boolean; loggedIn: boolean }>("GET", "/api/state");
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
  if (page === "setup") return <SetupPage onDone={refresh} />;
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
// Settings

type Notice = { kind: "ok" | "warn" | "error"; text: string } | null;
type Status = { running: boolean; configError?: string | null; error?: string | null };

function SettingsPage({ onLogout }: { onLogout: () => void }) {
  const [config, setConfig] = useState<Config | null>(null);
  const [saved, setSaved] = useState<string>("");
  const [notice, setNotice] = useState<Notice>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [busy, setBusy] = useState(false);

  const loadStatus = useCallback(() => {
    api<Status>("GET", "/api/status").then(setStatus).catch(() => setStatus(null));
  }, []);

  useEffect(() => {
    api<{ config: Config; error?: string }>("GET", "/api/config")
      .then((r) => {
        setConfig(r.config);
        setSaved(JSON.stringify(r.config));
        if (r.error) setNotice({ kind: "error", text: `${r.error} Saving will replace it.` });
      })
      .catch((e) => setNotice({ kind: "error", text: (e as Error).message }));
    loadStatus();
    const timer = setInterval(loadStatus, 5000);
    return () => clearInterval(timer);
  }, [loadStatus]);

  if (!config) return <main>{notice && <div className={`banner ${notice.kind}`}>{notice.text}</div>}</main>;

  const dirty = JSON.stringify(config) !== saved;
  const setDiscs = (patch: Partial<Config["discs"]>) =>
    setConfig({ ...config, discs: { ...config.discs, ...patch } });

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

  return (
    <main>
      <header>
        <div>
          <h1>familynovice</h1>
          <StatusLine status={status} />
        </div>
        <button onClick={logout}>Log out</button>
      </header>

      {notice && <div className={`banner ${notice.kind}`}>{notice.text}</div>}

      <section className="card">
        <h2>Discs</h2>
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
          hint="The first language the disc has is used."
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

      <div className="row" style={{ marginBottom: 24 }}>
        <button className="primary" onClick={save} disabled={busy || !dirty}>
          {busy ? "Saving…" : "Save and apply"}
        </button>
        {dirty && <span className="hint" style={{ margin: 0 }}>Unsaved changes</span>}
      </div>

      <ChangePassword />
    </main>
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
      <span className="dot on" /> Media center is running
      {status.configError ? ` — settings problem: ${status.configError}` : ""}
    </span>
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
    <details className="card">
      <summary>Change admin password</summary>
      <form onSubmit={submit} style={{ marginTop: 16 }}>
        {notice && <div className={`banner ${notice.kind}`}>{notice.text}</div>}
        <label className="field">
          <span>Current password</span>
          <input type="password" autoComplete="current-password" value={current} onChange={(e) => setCurrent(e.target.value)} />
        </label>
        <label className="field">
          <span>New password (at least 8 characters)</span>
          <input type="password" autoComplete="new-password" value={next} onChange={(e) => setNext(e.target.value)} />
        </label>
        <button disabled={!current || !next}>Change password</button>
      </form>
    </details>
  );
}

createRoot(document.getElementById("root")!).render(<App />);
