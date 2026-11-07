import { useCallback, useEffect, useState, type FormEvent } from "react";
import { createRoot } from "react-dom/client";
import { websiteDomain, type Config, type FolderSettings, type WebsiteSettings } from "../config";

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
        <h2>Home screen</h2>
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
      </section>

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

      <FoldersSection folders={config.folders} onChange={(folders) => setConfig({ ...config, folders })} />

      <WebsitesSection websites={config.websites} onChange={(websites) => setConfig({ ...config, websites })} />

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
      <h2>Folders</h2>
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
    onChange([...websites, { id, name: "", url: "https://", allow_subdomains: true }]);
  };

  return (
    <section className="card">
      <h2>Websites</h2>
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
      </label>
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
