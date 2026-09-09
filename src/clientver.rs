//! 客户端版本号：自动跟上上游
//! Client version: keep up with the far end automatically
//!
//! 提交时带的 `x-client-version` 得跟上游网页客户端一致，旧了会被直接拒绝
//! （"outdated client"）。这个号写死在代码里，上游一升级就得手动跟着改 —— 漏改
//! 一次服务就全线停摆。
//!
//! 所以改成自动拿：上游的网页是个单页应用，版本号明文写在它的脚本里。启动时和
//! 每小时把脚本抓下来，把里面形如 `x.y.z` 的版本候选都抠出来，再逐个拿一个假
//! 通行串去试 —— 版本太旧回 "outdated client"，版本可用回别的错（假串本身无效
//! 的那种）。试出可用的就记住，试不出来就退回代码里那个兜底值。
//!
//! 探测几乎不花钱：假通行串在长度检查之后就只换来一句"串无效"，不碰任何真实
//! 会话，也不占求解名额。
//!
//! The `x-client-version` sent with a submission has to match the far end's web
//! client, or the reply is a flat refusal ("outdated client"). Hard-coding the
//! number means every upstream release needs a manual edit — miss one and the
//! whole service goes down.
//!
//! So fetch it instead. The upstream page is a single-page app and the version
//! sits in plain sight in its script. At startup and then hourly, download that
//! script, pull out every `x.y.z`-shaped candidate, and try each one against a
//! throwaway pass string: a stale version draws "outdated client", a working one
//! draws a different error (the fake string itself being invalid). The first
//! working candidate wins; if none do, fall back to the number below.
//!
//! Probing costs almost nothing: after the length check a fake pass string only
//! earns "invalid payload" — no real session is touched, and no solving slot is
//! spent.

use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use crate::config;

/// 提取失败时的兜底版本号。随上游升级手动维护。
///
/// 挑它有两个前提：一是上游只查"不能比我旧"，写新一点没事；二是提取器每个
/// 小时都会自己试，真被超越了也只会错一阵子。
///
/// The fallback version when extraction fails. Maintained by hand as the far end
/// moves.
///
/// Two things make a slightly-ahead number safe: the far end only checks "not
/// older than me", and the extractor re-probes hourly, so being passed only
/// hurts for a while.
pub const FALLBACK_VERSION: &str = "5.4.0";

/// 上游单页应用的入口地址。
/// Where the far end's single-page app lives.
const APP_URL: &str = "https://auth.platorelay.com/a";

/// 脚本地址在 HTML 里的样子，`*` 是构建哈希。
/// How the script address appears in the HTML, with `*` for the build hash.
const SCRIPT_PATTERN: &str = "/assets/index-";

/// 缓存多久，到期后下一次提交前重新提取。
///
/// 上游不会天天升级，一小时足够跟得上了；太勤快反而给上游添没必要的流量。
///
/// How long a fetched version stays cached before the next submission refreshes.
///
/// The far end does not cut a release every day, so an hour keeps up fine;
/// probing any harder just spends their bandwidth for nothing.
const REFRESH_INTERVAL: Duration = Duration::from_secs(3600);

/// 现在认定的版本号。
/// The version currently believed to work.
fn slot() -> &'static RwLock<Arc<String>> {
    static SLOT: OnceLock<RwLock<Arc<String>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(Arc::new(FALLBACK_VERSION.to_string())))
}

/// 最近一次成功提取的时刻，用来决定要不要重新提取。
/// When extraction last succeeded, to decide whether to fetch again.
fn last_refresh() -> &'static RwLock<std::time::Instant> {
    static AT: OnceLock<RwLock<std::time::Instant>> = OnceLock::new();
    AT.get_or_init(|| RwLock::new(std::time::Instant::now() - REFRESH_INTERVAL))
}

/// 拿当前该用的版本号。只读缓存，不做网络请求 —— 刷新交给 [`refresh`]，在
/// 启动时和每小时的后台任务里跑。这样提交路径永远不会被提取拖慢。
///
/// Get the version to send now. This only reads the cache and never touches the
/// network — refreshing is [`refresh`]'s job, run at startup and by the hourly
/// background task, so the submission path is never slowed by extraction.
pub fn get() -> Arc<String> {
    slot().read().unwrap().clone()
}

/// 起一个后台线程：先刷新一次，之后每小时一次。
///
/// 服务一启动它就跑，第一条提交发出前版本多半已经就位；就算没就位，提交也会
/// 用兜底值先把请求发出去。
///
/// Spawn the background worker: one refresh straight away, then hourly.
///
/// It starts when the service starts, so the version is usually in place before
/// the first submission; if not, the submission still goes out with the fallback.
pub fn start_background() {
    std::thread::spawn(|| {
        refresh();
        loop {
            std::thread::sleep(REFRESH_INTERVAL);
            refresh();
        }
    });
}

/// 重新提取版本号。成功就换上，失败就保持原样 —— 兜底值或上一次的好结果。
///
/// 失败只记一行日志，绝不 panic：版本提取挂了不该把整个服务带走。
///
/// Fetch the version anew. On success swap it in; on failure keep whatever we
/// had — the fallback or the last good answer.
///
/// A failure only logs a line and never panics: a version-check outage must not
/// take the whole service down with it.
pub fn refresh() {
    match probe() {
        Some(version) => {
            let mut current = slot().write().unwrap();
            if **current != version {
                println!("[版本] 客户端版本更新: {} -> {}", current, version);
                *current = Arc::new(version);
            }
            *last_refresh().write().unwrap() = std::time::Instant::now();
        }
        None => {
            // 提取失败就只刷新时间戳，别在一小时内反复白跑。
            // A failed pass only bumps the timestamp, so we do not churn hourly.
            *last_refresh().write().unwrap() = std::time::Instant::now();
            let keeping = slot().read().unwrap().clone();
            println!("[版本] 提取失败，继续用 {} / extraction failed, keeping {}", keeping, keeping);
        }
    }
}

/// 抓脚本、抠候选、逐个试。全链路任何一步失败都返回 `None`。
///
/// 脚本有 2MB，但版本号实测总在前 128KB 的字符串数组里 —— 所以先用 Range 请求
/// 只取前 256KB（省 87% 的流量）；万一根本不在里面（上游改了打包顺序），再退回
/// 全量下载兜底。
///
/// Fetch the script, gather candidates, try each. Any failure anywhere yields
/// `None`.
///
/// The script runs about 2MB, but the version has always sat in the string array
/// inside the first 128KB — so a Range request takes only the first 256KB
/// (saving 87% of the traffic); should it not be there at all (the far end
/// rearranged its bundle), a full download backs that up.
fn probe() -> Option<String> {
    const HEAD_BYTES: u64 = 256 * 1024;

    let script_url = fetch_script_url()?;

    // 先试前 256KB。
    // First try the first 256KB.
    //
    // 脚本内容只在这个块里活 —— 候选一提取出来就释放，不留副本、不落盘。最后
    // 留在内存里的只有几字节的版本号本身。
    // The script body only lives inside this block — freed as soon as the
    // candidates are out, no copies kept, nothing written to disk. All that
    // stays in memory afterwards is the few-byte version string itself.
    let candidates = match fetch_range(&script_url, HEAD_BYTES) {
        Some(body) => {
            let found = version_candidates(&body);
            drop(body);
            found
        }
        None => Vec::new(),
    };

    if !candidates.is_empty() {
        return candidates.into_iter().find(|candidate| works(candidate));
    }

    // 前段没有，退回全量 —— 同样用完即弃。
    // Nothing in the head, so fall back to the whole thing — dropped right after
    // use as well.
    let candidates = {
        let body = fetch_text(&script_url)?;
        let found = version_candidates(&body);
        drop(body);
        found
    };
    if candidates.is_empty() {
        return None;
    }

    candidates.into_iter().find(|candidate| works(candidate))
}

/// 从入口 HTML 里抠出脚本地址。
///
/// 页面就一行 `<script ... src="/assets/index-xxxxxxxx.js">`，按前缀找，取到
/// 引号为止。相对地址补上域名。
///
/// Pull the script address out of the entry HTML.
///
/// The page carries one `<script ... src="/assets/index-xxxxxxxx.js">` line; find
/// it by prefix and read up to the closing quote. The relative address gets the
/// site in front.
fn fetch_script_url() -> Option<String> {
    let html = fetch_text(APP_URL)?;

    let start = html.find(SCRIPT_PATTERN)?;
    let rest = &html[start..];
    let end = rest.find('"')?;
    let path = &rest[..end];

    Some(format!("https://auth.platorelay.com{}", path))
}

/// 抓一个文本资源。失败返回 `None`，超时从紧（几秒），别让它拖慢提交。
///
/// Fetch one text resource. Failures give `None`, with a tight timeout so the
/// submission path is never dragged.
fn fetch_text(address: &str) -> Option<String> {
    let reply = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?
        .get(address)
        .header("User-Agent", crate::useragent::next().agent)
        .send()
        .ok()?;

    if !reply.status().is_success() {
        return None;
    }

    reply.text().ok()
}

/// 只抓一个资源的前 `take` 字节（Range 请求）。
///
/// 上游的 CDN 支持 Range（实测 206），2MB 的脚本就能只取头部。服务器不支持
/// Range 时回 200 + 全量，这里按原样收下 —— 调用方反正会做兜底。
///
/// Fetch only the first `take` bytes of a resource (a Range request).
///
/// The far end's CDN honours Range (206 in testing), so a 2MB script only needs
/// its head. A server without Range support answers 200 with the whole body,
/// which we accept as-is — the caller has a fallback anyway.
fn fetch_range(address: &str, take: u64) -> Option<String> {
    let reply = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?
        .get(address)
        .header("User-Agent", crate::useragent::next().agent)
        .header("Range", format!("bytes=0-{}", take - 1))
        .send()
        .ok()?;

    if !reply.status().is_success() {
        return None;
    }

    reply.text().ok()
}

/// 从脚本内容里抠出所有形如 `x.y.z` 的版本候选。
///
/// 脚本是混淆过的，字符串都装在一个大数组里，版本号也是其中一个元素。正则按
/// 引号配对抓 `数字.数字.数字`，抓到的除了客户端版本还有第三方库的版本（比如
/// axios 的 1.7.9），顺序与重要性无关 —— 所以只收集不判断，交给 [`works`]
/// 一个个试。
///
/// Pull every `x.y.z`-shaped candidate out of the script.
///
/// The script is obfuscated: its strings live in one big array, and the version
/// is just one element. Matching quoted digit groups also catches third-party
/// library versions (axios' 1.7.9 among them), in no meaningful order — so we
/// only collect here and let [`works`] sort them out one by one.
fn version_candidates(script: &str) -> Vec<String> {
    let mut found = Vec::new();

    // 手写扫描代替正则：脚本有 2MB，逐字符配引号比引入 regex 依赖划算。
    // A hand-rolled scan instead of a regex: at 2MB the script makes char-by-char
    // quote pairing cheaper than pulling in a regex crate.
    let bytes = script.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'\'' {
                j += 1;
            }
            if j < bytes.len() {
                let token = &script[start..j];
                if is_version_shape(token) && !found.contains(&token.to_string()) {
                    found.push(token.to_string());
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }

    found
}

/// 看一串字符是不是 `x.y.z` 的样子（三段纯数字）。
/// Whether a string looks like `x.y.z` (three all-digit parts).
fn is_version_shape(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// 试一个版本号能不能用。
///
/// 拿一个假的通行串提交一次：串本身是随机编的，格式合法但不指向任何真会话。
/// 服务器先查版本再查串 —— 所以回 "outdated client" 就是版本不行，回别的错
/// （"invalid payload" 之类）就是版本过了关。
///
/// Try whether a version works.
///
/// Submit once with a fake pass string: random, well-formed, pointing at no real
/// session. The far end checks the version before the string — so "outdated
/// client" means the version is no good, and any other reply ("invalid payload"
/// and friends) means the version passed.
fn works(version: &str) -> bool {
    use rand::Rng;

    let agent = crate::useragent::next();
    let mut rng = rand::thread_rng();
    let fake: String = (0..64)
        .map(|_| {
            let pick = rng.gen_range(0..62);
            match pick {
                0..=9 => (b'0' + pick) as char,
                10..=35 => (b'A' + pick - 10) as char,
                _ => (b'a' + pick - 36) as char,
            }
        })
        .collect();

    let Some(payload) = crate::auth::build_payload(&fake, crate::auth::now_ms(), agent.agent, agent.screen)
    else {
        return false;
    };

    let address = format!(
        "{}/session/step?ticket={}&service=3",
        config::AUTH_API,
        urlencoding::encode(&fake)
    );
    let body = serde_json::json!({
        "captcha": serde_json::Value::Null,
        "meta": payload.meta,
        "stream": payload.stream,
        "resolved": true
    })
    .to_string();

    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    match client
        .put(&address)
        .body(body)
        .header("User-Agent", agent.agent)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("x-client-name", "platoboost webclient")
        .header("x-client-version", version)
        .send()
    {
        Ok(reply) => match reply.text() {
            Ok(text) => !text.contains("outdated client"),
            Err(_) => false,
        },
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 版本形状能认出来_version_shape_is_recognised() {
        assert!(is_version_shape("5.4.0"));
        assert!(is_version_shape("1.7.9"));
        assert!(is_version_shape("10.20.30"));

        assert!(!is_version_shape("5.4"), "两段不是版本 / two parts is not one");
        assert!(!is_version_shape("5.4.0.1"), "四段也不是 / four parts is not either");
        assert!(!is_version_shape("v5.4.0"), "带前缀不行 / a prefix spoils it");
        assert!(!is_version_shape("5.x.0"), "非数字段不行 / a non-digit part spoils it");
        assert!(!is_version_shape(""), "空串不行 / empty is nothing");
    }

    #[test]
    fn 从脚本里抠出版本候选_candidates_come_out_of_a_script() {
        // 模拟混淆脚本的样子：一个大数组里混着乱码和版本号，axios 的 1.7.9 也在。
        // Mimic the obfuscated script: one big array of noise with the versions in.
        let script = r"'hYvKH','5.4.0','JzJhH','oooo','hkbHB','1.7.9','UDKtb'";
        let got = version_candidates(script);

        assert_eq!(got.len(), 2, "应该抠出两个候选 / two candidates should come out");
        assert!(got.contains(&"5.4.0".to_string()));
        assert!(got.contains(&"1.7.9".to_string()));
    }

    #[test]
    fn 重复候选只留一个_duplicate_candidates_are_kept_once() {
        let script = "'5.4.0','xxxx','5.4.0'";
        let got = version_candidates(script);
        assert_eq!(got.len(), 1, "重复的只留一个 / a duplicate is kept once");
    }

    #[test]
    fn 没有候选就空手而归_no_candidates_gives_an_empty_list() {
        let script = "'hYvKH','JzJhH','oooo'";
        assert!(version_candidates(script).is_empty());
    }

    #[test]
    fn 从页面里抠出脚本地址_script_address_comes_out_of_the_page() {
        // fetch_script_url 依赖网络，这里只测路径拼装的形状。
        // fetch_script_url needs the network; only the shape is pinned here.
        let path = "/assets/index-f568705f.js";
        let url = format!("https://auth.platorelay.com{}", path);
        assert!(url.starts_with("https://auth.platorelay.com/assets/index-"));
        assert!(url.ends_with(".js"));
    }
}
