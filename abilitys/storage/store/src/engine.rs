//! 后端分派。调用方不出现本模块名。

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use fs4::fs_std::FileExt;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::Connection;
use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::codec::{b64, decode, decode_b64, encode};
use crate::loc::{resolve_path, roots_from_config, space_root, validate_owner};
use crate::types::{AcquireOut, BlobOut, Codec, GetOut, ListOut, OkOut, Space, StatOut};
use ability_support::{Error, Result};

const PART_SUFFIX: &str = ".part";
const SQLITE_FILE: &str = "kv.sqlite";

pub struct Engine {
    data_root: PathBuf,
    temp_root: PathBuf,
    watch_tx: mpsc::UnboundedSender<Value>,
    watch_rx: Mutex<Option<mpsc::UnboundedReceiver<Value>>>,
    watcher: Mutex<Option<RecommendedWatcher>>,
    locks: Mutex<HashMap<String, File>>,
    sqlite: Mutex<HashMap<PathBuf, Connection>>,
}

impl Engine {
    pub fn from_config(config: &Value) -> Result<Arc<Self>> {
        let (data_root, temp_root) = roots_from_config(config);
        fs::create_dir_all(&data_root).map_err(|e| Error::io(e.to_string()))?;
        fs::create_dir_all(&temp_root).map_err(|e| Error::io(e.to_string()))?;
        let (tx, rx) = mpsc::unbounded_channel();
        let watch = config.get("watch").and_then(Value::as_bool).unwrap_or(true);
        let eng = Arc::new(Self {
            data_root,
            temp_root,
            watch_tx: tx,
            watch_rx: Mutex::new(Some(rx)),
            watcher: Mutex::new(None),
            locks: Mutex::new(HashMap::new()),
            sqlite: Mutex::new(HashMap::new()),
        });
        if watch {
            eng.start_watch()?;
        }
        Ok(eng)
    }

    pub fn take_watch_rx(&self) -> Option<mpsc::UnboundedReceiver<Value>> {
        self.watch_rx.lock().expect("watch_rx").take()
    }

    pub fn stop_watch(&self) {
        *self.watcher.lock().expect("watcher") = None;
    }

    fn start_watch(&self) -> Result<()> {
        let tx = self.watch_tx.clone();
        let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                let op = format!("{:?}", ev.kind);
                for p in ev.paths {
                    let _ = tx.send(serde_json::json!({
                        "path": p.to_string_lossy(),
                        "op": op,
                    }));
                }
            }
        })
        .map_err(|e| Error::failed(format!("文件监听启动失败: {e}")))?;
        w.watch(&self.data_root, RecursiveMode::Recursive)
            .map_err(|e| Error::failed(format!("监听 persist 失败: {e}")))?;
        w.watch(&self.temp_root, RecursiveMode::Recursive)
            .map_err(|e| Error::failed(format!("监听 temp 失败: {e}")))?;
        *self.watcher.lock().expect("watcher") = Some(w);
        Ok(())
    }

    fn root(&self, space: Space) -> PathBuf {
        space_root(space, &self.data_root, &self.temp_root)
    }

    fn resolve(&self, space: Space, owner: &str, key: &str) -> Result<PathBuf> {
        resolve_path(&self.root(space), owner, key)
    }

    fn codec_of(key: &str, codec: Option<&str>) -> Result<Codec> {
        match codec {
            Some(s) if !s.is_empty() => Codec::parse(s).map_err(Error::invalid_args),
            _ => Ok(Codec::from_key(key)),
        }
    }

    pub fn get(&self, space: Space, owner: &str, key: &str, codec: Option<&str>) -> Result<GetOut> {
        let codec = Self::codec_of(key, codec)?;
        if codec == Codec::Sqlite {
            let Some(bytes) = self.sqlite_get(space, owner, key)? else {
                return Ok(GetOut {
                    exists: false,
                    value: None,
                    codec: codec.as_str().into(),
                });
            };
            return Ok(GetOut {
                exists: true,
                value: Some(decode(Codec::Json, &bytes)?),
                codec: codec.as_str().into(),
            });
        }
        let path = self.resolve(space, owner, key)?;
        if !path.is_file() {
            return Ok(GetOut {
                exists: false,
                value: None,
                codec: codec.as_str().into(),
            });
        }
        let bytes = fs::read(&path).map_err(|e| Error::io(e.to_string()))?;
        Ok(GetOut {
            exists: true,
            value: Some(decode(codec, &bytes)?),
            codec: codec.as_str().into(),
        })
    }

    pub fn put(
        &self,
        space: Space,
        owner: &str,
        key: &str,
        value: &Value,
        codec: Option<&str>,
    ) -> Result<OkOut> {
        let codec = Self::codec_of(key, codec)?;
        let bytes = encode(codec, value)?;
        if codec == Codec::Sqlite {
            self.sqlite_put(space, owner, key, &bytes)?;
            return Ok(OkOut { ok: true });
        }
        let path = self.resolve(space, owner, key)?;
        atomic_write(&path, &bytes)?;
        Ok(OkOut { ok: true })
    }

    pub fn merge(
        &self,
        space: Space,
        owner: &str,
        key: &str,
        patch: Value,
        codec: Option<&str>,
    ) -> Result<GetOut> {
        let cur = self.get(space, owner, key, codec)?;
        let mut base = cur.value.unwrap_or(Value::Object(Default::default()));
        merge_value(&mut base, patch);
        self.put(space, owner, key, &base, codec)?;
        Ok(GetOut {
            exists: true,
            value: Some(base),
            codec: cur.codec,
        })
    }

    pub fn delete(
        &self,
        space: Space,
        owner: &str,
        key: &str,
        codec: Option<&str>,
    ) -> Result<OkOut> {
        let codec = Self::codec_of(key, codec)?;
        if codec == Codec::Sqlite {
            self.sqlite_del(space, owner, key)?;
            return Ok(OkOut { ok: true });
        }
        let path = self.resolve(space, owner, key)?;
        if path.is_file() {
            fs::remove_file(&path).map_err(|e| Error::io(e.to_string()))?;
        }
        Ok(OkOut { ok: true })
    }

    pub fn exists(
        &self,
        space: Space,
        owner: &str,
        key: &str,
        codec: Option<&str>,
    ) -> Result<StatOut> {
        Ok(self.stat(space, owner, key, codec)?)
    }

    pub fn stat(
        &self,
        space: Space,
        owner: &str,
        key: &str,
        codec: Option<&str>,
    ) -> Result<StatOut> {
        let codec = Self::codec_of(key, codec)?;
        if codec == Codec::Sqlite {
            let Some(b) = self.sqlite_get(space, owner, key)? else {
                return Ok(StatOut {
                    exists: false,
                    len: 0,
                });
            };
            return Ok(StatOut {
                exists: true,
                len: b.len() as u64,
            });
        }
        let path = self.resolve(space, owner, key)?;
        match fs::metadata(&path) {
            Ok(m) if m.is_file() => Ok(StatOut {
                exists: true,
                len: m.len(),
            }),
            _ => Ok(StatOut {
                exists: false,
                len: 0,
            }),
        }
    }

    pub fn list(&self, space: Space, owner: &str, prefix: &str) -> Result<ListOut> {
        validate_owner(owner)?;
        let mut keys = Vec::new();
        let base = self.root(space).join(owner);
        if base.is_dir() {
            walk_keys(&base, &base, prefix, &mut keys)?;
        }
        if let Ok(sqlite_keys) = self.sqlite_list(space, owner, prefix) {
            for k in sqlite_keys {
                if !keys.iter().any(|x| x == &k) {
                    keys.push(k);
                }
            }
        }
        keys.sort();
        Ok(ListOut { keys })
    }

    pub fn purge(&self, space: Space, owner: &str, prefix: &str) -> Result<OkOut> {
        let listed = self.list(space, owner, prefix)?;
        for k in listed.keys {
            let _ = self.delete(space, owner, &k, None);
            let _ = self.delete(space, owner, &k, Some("sqlite"));
        }
        Ok(OkOut { ok: true })
    }

    pub fn blob_get(
        &self,
        space: Space,
        owner: &str,
        key: &str,
        offset: u64,
        len: Option<u64>,
    ) -> Result<BlobOut> {
        let path = self.resolve(space, owner, key)?;
        if !path.is_file() {
            return Ok(BlobOut {
                data: String::new(),
                eof: true,
                len: 0,
            });
        }
        let mut f = File::open(&path).map_err(|e| Error::io(e.to_string()))?;
        let total = f.metadata().map_err(|e| Error::io(e.to_string()))?.len();
        if offset >= total {
            return Ok(BlobOut {
                data: String::new(),
                eof: true,
                len: total,
            });
        }
        f.seek(SeekFrom::Start(offset))
            .map_err(|e| Error::io(e.to_string()))?;
        let take = len
            .unwrap_or(total.saturating_sub(offset))
            .min(total - offset);
        let mut buf = vec![0u8; take as usize];
        let n = f.read(&mut buf).map_err(|e| Error::io(e.to_string()))?;
        buf.truncate(n);
        let eof = offset + n as u64 >= total;
        Ok(BlobOut {
            data: b64(&buf),
            eof,
            len: total,
        })
    }

    pub fn blob_put(
        &self,
        space: Space,
        owner: &str,
        key: &str,
        data_b64: &str,
        append: bool,
    ) -> Result<OkOut> {
        let bytes = decode_b64(data_b64)?;
        let path = self.resolve(space, owner, key)?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| Error::io(e.to_string()))?;
        }
        if append && path.is_file() {
            let mut f = OpenOptions::new()
                .append(true)
                .open(&path)
                .map_err(|e| Error::io(e.to_string()))?;
            f.write_all(&bytes).map_err(|e| Error::io(e.to_string()))?;
            f.sync_all().map_err(|e| Error::io(e.to_string()))?;
        } else {
            atomic_write(&path, &bytes)?;
        }
        Ok(OkOut { ok: true })
    }

    pub fn blob_write(
        &self,
        space: Space,
        owner: &str,
        key: &str,
        offset: u64,
        data_b64: &str,
        last: bool,
    ) -> Result<OkOut> {
        let bytes = decode_b64(data_b64)?;
        let final_path = self.resolve(space, owner, key)?;
        let part = part_path(&final_path);
        if let Some(dir) = final_path.parent() {
            fs::create_dir_all(dir).map_err(|e| Error::io(e.to_string()))?;
        }
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .open(&part)
            .map_err(|e| Error::io(e.to_string()))?;
        f.seek(SeekFrom::Start(offset))
            .map_err(|e| Error::io(e.to_string()))?;
        f.write_all(&bytes).map_err(|e| Error::io(e.to_string()))?;
        f.sync_all().map_err(|e| Error::io(e.to_string()))?;
        drop(f);
        if last {
            fs::rename(&part, &final_path).map_err(|e| Error::io(e.to_string()))?;
        }
        Ok(OkOut { ok: true })
    }

    pub fn acquire(&self, space: Space, owner: &str, key: &str) -> Result<AcquireOut> {
        let path = self.resolve(space, owner, key)?;
        let lock_path = path.with_extension(format!(
            "{}lock",
            path.extension()
                .map(|e| format!("{}.", e.to_string_lossy()))
                .unwrap_or_default()
        ));
        if let Some(dir) = lock_path.parent() {
            fs::create_dir_all(dir).map_err(|e| Error::io(e.to_string()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|e| Error::io(e.to_string()))?;
        file.lock_exclusive()
            .map_err(|e| Error::io(format!("获取锁失败: {e}")))?;
        let token = Uuid::new_v4().to_string();
        self.locks
            .lock()
            .expect("locks")
            .insert(token.clone(), file);
        Ok(AcquireOut { token })
    }

    pub fn release(&self, token: &str) -> Result<OkOut> {
        if let Some(f) = self.locks.lock().expect("locks").remove(token) {
            let _ = f.unlock();
        }
        Ok(OkOut { ok: true })
    }

    fn sqlite_path(&self, space: Space, owner: &str) -> Result<PathBuf> {
        validate_owner(owner)?;
        Ok(self.root(space).join(owner).join(SQLITE_FILE))
    }

    fn with_sqlite<R>(
        &self,
        space: Space,
        owner: &str,
        f: impl FnOnce(&Connection) -> rusqlite::Result<R>,
    ) -> Result<R> {
        let path = self.sqlite_path(space, owner)?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| Error::io(e.to_string()))?;
        }
        let mut map = self.sqlite.lock().expect("sqlite");
        if !map.contains_key(&path) {
            let conn = Connection::open(&path).map_err(|e| Error::io(e.to_string()))?;
            conn.execute(
                "CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v BLOB NOT NULL)",
                [],
            )
            .map_err(|e| Error::io(e.to_string()))?;
            map.insert(path.clone(), conn);
        }
        let conn = map.get(&path).expect("conn");
        f(conn).map_err(|e| Error::io(e.to_string()))
    }

    fn sqlite_get(&self, space: Space, owner: &str, key: &str) -> Result<Option<Vec<u8>>> {
        self.with_sqlite(space, owner, |c| {
            let mut stmt = c.prepare("SELECT v FROM kv WHERE k = ?1")?;
            let mut rows = stmt.query([key])?;
            match rows.next()? {
                Some(r) => Ok(Some(r.get(0)?)),
                None => Ok(None),
            }
        })
    }

    fn sqlite_put(&self, space: Space, owner: &str, key: &str, v: &[u8]) -> Result<()> {
        self.with_sqlite(space, owner, |c| {
            c.execute(
                "INSERT INTO kv(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=excluded.v",
                rusqlite::params![key, v],
            )?;
            Ok(())
        })
    }

    fn sqlite_del(&self, space: Space, owner: &str, key: &str) -> Result<()> {
        let path = self.sqlite_path(space, owner)?;
        if !path.is_file() {
            return Ok(());
        }
        self.with_sqlite(space, owner, |c| {
            c.execute("DELETE FROM kv WHERE k = ?1", [key])?;
            Ok(())
        })
    }

    fn sqlite_list(&self, space: Space, owner: &str, prefix: &str) -> Result<Vec<String>> {
        let path = self.sqlite_path(space, owner)?;
        if !path.is_file() {
            return Ok(Vec::new());
        }
        self.with_sqlite(space, owner, |c| {
            let mut stmt = c.prepare("SELECT k FROM kv WHERE k LIKE ?1")?;
            let pat = format!("{prefix}%");
            let rows = stmt.query_map([pat], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for k in rows {
                out.push(k?);
            }
            Ok(out)
        })
    }
}

fn part_path(final_path: &Path) -> PathBuf {
    let mut os = final_path.as_os_str().to_os_string();
    os.push(PART_SUFFIX);
    PathBuf::from(os)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| Error::io(e.to_string()))?;
    }
    let tmp = path.with_extension(format!(
        "{}tmp",
        path.extension()
            .map(|e| format!("{}.", e.to_string_lossy()))
            .unwrap_or_default()
    ));
    {
        let mut f = File::create(&tmp).map_err(|e| Error::io(e.to_string()))?;
        f.write_all(bytes).map_err(|e| Error::io(e.to_string()))?;
        f.sync_all().map_err(|e| Error::io(e.to_string()))?;
    }
    fs::rename(&tmp, path).map_err(|e| Error::io(e.to_string()))?;
    Ok(())
}

fn walk_keys(base: &Path, dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<()> {
    let rd = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    for ent in rd {
        let ent = ent.map_err(|e| Error::io(e.to_string()))?;
        let p = ent.path();
        if p.is_dir() {
            walk_keys(base, &p, prefix, out)?;
            continue;
        }
        if p.extension().and_then(|e| e.to_str()) == Some("tmp") {
            continue;
        }
        if p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(PART_SUFFIX) || n.ends_with(".lock") || n == SQLITE_FILE)
            .unwrap_or(false)
        {
            continue;
        }
        let rel = p.strip_prefix(base).unwrap_or(&p);
        let key = rel.to_string_lossy().replace('\\', "/");
        if prefix.is_empty() || key.starts_with(prefix) {
            out.push(key);
        }
    }
    Ok(())
}

fn merge_value(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                merge_value(a.entry(k).or_insert(Value::Null), v);
            }
        }
        (slot, v) => *slot = v,
    }
}
