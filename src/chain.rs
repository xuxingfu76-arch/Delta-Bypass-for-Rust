//! 从链接一路走到钥匙
//! Walk from a link all the way to a key
//!
//! 整条路是这样的：
//!
//! ```text
//! 链接 → 问情况 → 提交 → 解出下一段 → 再提交 → … → 拿到钥匙
//! link → ask about it → submit → decode next → submit again → … → key
//! ```
//!
//! 三个省时间的做法：
//!
//! 1. **第一轮三件事同时发。** 提交不等 meta（服务编号先用兜底值），和"问情况"
//!    "查钥匙"并行 —— 省一趟串行往返。
//!
//! 2. **问情况和查钥匙一起干。** 问服务器情况的那段时间，顺便看看这链接是不是已经
//!    完成过了。
//!
//! 3. **提交和查钥匙叠着做。** 提交发出去 50 毫秒后就开始查钥匙，不等提交回话 ——
//!    省掉一趟来回。
//!
//! 注：上游已取消图形验证码环节（step 的 `captcha` 字段不校验），所以这里不再有
//! 识别步骤。
//!
//! The whole route looks like this:
//!
//! ```text
//! link → ask about it → submit → decode next → submit again → … → key
//! ```
//!
//! Three things that save time:
//!
//! 1. **Fire three things at once on the first round.** The submission does not
//!    wait for meta (it starts with the fallback service number), running in
//!    parallel with the link query and the key check — one serial round trip
//!    saved.
//!
//! 2. **Ask about it and check for the key at once.** While asking the server about
//!    the link, also see whether it has already been completed.
//!
//! 3. **Overlap submitting with checking for the key.** Checking starts 50
//!    milliseconds after the submission goes out, without waiting for its reply — one
//!    round trip saved.
//!
//! Note: the far end dropped the picture captcha (the step `captcha` field is not
//! checked), so there is no recognition step any more.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::auth;
use crate::config;
use crate::timing::{steps, Timer};

/// 服务编号。上游告诉我们用哪个；问不出来就用这个兜底。
/// The service number. The far end tells us which to use; this is the fallback when it
/// will not say.
const FALLBACK_SERVICE: i64 = 3;

// ---------------------------------------------------------------------------
// 等待余量：自己会调 / The waiting margin: it tunes itself
// ---------------------------------------------------------------------------

/// 当前多留的余量（毫秒）。
///
/// 为什么要留：两次提交必须隔 5 秒，但我们只知道自己什么时候发出去，对面看的是它
/// 什么时候收到。网络快慢有波动，掐着 5 秒发很可能 4.98 秒就到了，对面就判"太快"，
/// 那要罚等 2 秒 —— 比多留 0.2 秒亏多了。
///
/// 怎么调：一直顺就慢慢往下减，被拒一次立刻往上加。这样它会自己找到当前网络下最
/// 合适的值。
///
/// The extra margin currently in use, in milliseconds.
///
/// Why have one: two submissions must be 5 seconds apart, but we only know when we
/// sent ours while the far end goes by when it arrived. Network speed wobbles, so
/// sending at exactly 5 seconds may well arrive at 4.98, the far end calls it "too
/// fast", and we get a 2-second penalty — far worse than an extra 0.2 seconds.
///
/// How it tunes: shrink slowly while things go well, jump up the moment we get
/// refused. That way it finds the right value for the current network by itself.
static MARGIN_MS: AtomicU64 = AtomicU64::new(config::GAP_MARGIN_START.as_millis() as u64);

/// 连续顺利了几次。
/// How many good runs in a row so far.
static GOOD_RUNS: AtomicU64 = AtomicU64::new(0);

/// 现在留多少余量。
/// How much margin to leave right now.
fn margin() -> Duration {
    Duration::from_millis(MARGIN_MS.load(Ordering::Relaxed))
}

/// 这次很顺利。攒够几次就把余量减一点。
/// That went well. After enough good runs, shave the margin a little.
fn note_good() {
    let runs = GOOD_RUNS.fetch_add(1, Ordering::Relaxed) + 1;
    if runs < config::CLEAN_STEPS_TO_RELAX {
        return;
    }

    GOOD_RUNS.store(0, Ordering::Relaxed);

    let now = MARGIN_MS.load(Ordering::Relaxed);
    let less = now
        .saturating_sub(config::GAP_MARGIN_STEP_DOWN.as_millis() as u64)
        .max(config::GAP_MARGIN_MIN.as_millis() as u64);

    MARGIN_MS.store(less, Ordering::Relaxed);
}

/// 被判"太快"了。余量马上加上去，好运计数清零。
/// We were told "too fast". Put the margin up straight away and reset the good-run
/// count.
fn note_too_fast() {
    GOOD_RUNS.store(0, Ordering::Relaxed);

    let now = MARGIN_MS.load(Ordering::Relaxed);
    let more = (now + config::GAP_MARGIN_STEP_UP.as_millis() as u64)
        .min(config::GAP_MARGIN_MAX.as_millis() as u64);

    MARGIN_MS.store(more, Ordering::Relaxed);
}

/// verbose 模式下才打印。
/// Only print when talking.
fn say(talk: bool, line: &str) {
    if talk {
        println!("{}", line);
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
}

// ---------------------------------------------------------------------------
// 问链接情况 / Asking about the link
// ---------------------------------------------------------------------------

/// 问出来的链接情况。
/// What we learned about the link.
struct LinkInfo {
    /// 用哪个服务编号。问不出来就是空。
    /// Which service number to use. Empty if it would not say.
    service: Option<i64>,
    /// 要过几关。问不出来就是空。
    /// How many checkpoints there are. Empty if it would not say.
    checkpoints: Option<i64>,
    /// 这链接还能不能用。
    /// Whether the link is still usable.
    alive: bool,
    /// 不能用的原因。
    /// Why it is not usable.
    dead_reason: Option<String>,
}

/// 问服务器这个链接的情况。
///
/// 一次问清三件事：用哪个服务、要过几关、还有没有效。原来要问两次，合成一次省一趟
/// 来回。
///
/// Ask the server about this link.
///
/// Three things in one go: which service, how many checkpoints, and whether it is
/// still valid. This used to take two questions; merging them saves a round trip.
fn ask_about(ticket: &str, talk: bool) -> LinkInfo {
    let reply = auth::session_info(ticket);

    // 先看有没有明确说"这链接废了"。
    // First see whether it plainly says the link is dead.
    let said_no = reply.get("success").and_then(Value::as_bool) == Some(false);
    let just_a_hiccup = reply.get("transient").and_then(Value::as_bool) == Some(true);

    if said_no && !just_a_hiccup {
        let words = reply
            .get("message")
            .or_else(|| reply.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let lower = words.to_lowercase();

        if auth::DEAD_LINK_WORDS.iter().any(|w| lower.contains(w)) {
            say(talk, &format!("  [会话] 链接已失效: {}", words));
            return LinkInfo {
                service: None,
                checkpoints: None,
                alive: false,
                dead_reason: Some(words.to_string()),
            };
        }
    }

    // 再从回话里把服务编号和关卡数掏出来。
    // Then dig the service number and checkpoint count out of the reply.
    let body = reply.get("data").unwrap_or(&reply);

    if let Some(profile) = body
        .get("activeRevenueProfile")
        .and_then(Value::as_object)
    {
        if let Some(service) = profile.get("service").and_then(Value::as_i64) {
            let checkpoints = profile
                .get("checkpointCount")
                .and_then(Value::as_i64)
                .filter(|&n| n > 0);

            let hours = body.get("duration").and_then(Value::as_str).unwrap_or("?");
            say(
                talk,
                &format!(
                    "  [会话] 服务ID={} 检查点数={} 有效期={}小时",
                    service,
                    checkpoints.map(|n| n.to_string()).unwrap_or_else(|| "?".into()),
                    hours
                ),
            );

            return LinkInfo {
                service: Some(service),
                checkpoints,
                alive: true,
                dead_reason: None,
            };
        }
    }

    // 什么都没问出来。当能用处理，别把好链接判死。
    // Nothing came out. Treat it as usable rather than writing off a good link.
    LinkInfo {
        service: None,
        checkpoints: None,
        alive: true,
        dead_reason: None,
    }
}

/// [`take_in_info`] 的产出。
/// What [`take_in_info`] hands back.
struct LinkTake {
    /// 死链原因。活链接是空。
    /// Why the link is dead. Empty for a live one.
    dead: Option<String>,
    /// meta 报的服务编号。
    /// The service number meta reported.
    service: Option<i64>,
}

/// 收下 meta 的结果：死链给出原因，关卡数多就把轮数上限放宽，顺带传出服务编号。
///
/// Take in the meta result: a dead link carries its reason, more checkpoints widen
/// the round cap, and the service number is passed along.
fn take_in_info(info: LinkInfo, talk: bool, rounds_allowed: &mut usize) -> LinkTake {
    if !info.alive {
        return LinkTake {
            dead: info.dead_reason,
            service: None,
        };
    }

    // 关卡数比预设多，就把轮数放宽。留一轮余量，因为最后一关之后还要取钥匙。
    // More checkpoints than assumed, so loosen the round cap. One spare round is
    // added, since the key still has to be fetched after the last checkpoint.
    if let Some(count) = info.checkpoints {
        let need = (count + 1) as usize;
        if need > *rounds_allowed {
            *rounds_allowed = need.min(config::MAX_ROUNDS_HARD_CAP);
            say(
                talk,
                &format!("  [轮数] 检查点数={}，上限调整为{}轮", count, rounds_allowed),
            );
        }
    }

    LinkTake {
        dead: None,
        service: info.service,
    }
}

// ---------------------------------------------------------------------------
// 查钥匙 / Checking for the key
// ---------------------------------------------------------------------------

/// 查一次钥匙好了没。
/// Check once whether the key is ready.
fn check_key(ticket: &str, talk: bool, timer: Option<&mut Timer>) -> Option<String> {
    let mut timer = timer;

    if let Some(t) = timer.as_mut() {
        t.start(steps::POLL);
    }
    let reply = auth::session_status(ticket);
    if let Some(t) = timer.as_mut() {
        t.stop();
    }

    let body = reply.get("data").unwrap_or(&reply);
    let key = body.get("key").and_then(Value::as_str).unwrap_or("");

    // 服务器用 KEY_NOT_FOUND 表示"还没好"，那不是钥匙。
    // The server uses KEY_NOT_FOUND to mean "not ready yet", which is not a key.
    if key.is_empty() || key == "KEY_NOT_FOUND" {
        return None;
    }

    say(talk, &format!("  [密钥] 已获取: {}", key));
    Some(key.to_string())
}

/// 反复查钥匙，拿到就返回。
/// Check for the key repeatedly, returning as soon as it turns up.
fn wait_for_key(ticket: &str, talk: bool, timer: &mut Timer) -> Option<String> {
    for round in 0..config::POLL_MAX_ATTEMPTS {
        if let Some(key) = check_key(ticket, talk, Some(timer)) {
            return Some(key);
        }
        if round + 1 < config::POLL_MAX_ATTEMPTS {
            std::thread::sleep(config::POLL_INTERVAL);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 提交 / Submitting
// ---------------------------------------------------------------------------

/// 看服务器是不是在说"太快了"。
/// See whether the server is saying "you are going too fast".
fn says_too_fast(reply: &Value) -> bool {
    let mut all = String::new();
    for field in ["message", "error", "detail"] {
        if let Some(text) = reply.get(field).and_then(Value::as_str) {
            all.push_str(text);
            all.push(' ');
        }
    }
    let lower = all.to_lowercase();
    lower.contains("too fast") || lower.contains("slow down") || lower.contains("too many")
}

/// 看服务器是不是在说"服务编号不对"。
///
/// 兜底服务编号（Delta=3）被别的链接拒绝时靠它识别，好换 meta 报的编号补一发。
///
/// See whether the server is saying the service number is wrong.
///
/// This spots the fallback service number (Delta = 3) being refused by another
/// link, so a follow-up can be made with the number meta reported.
fn says_wrong_service(reply: &Value) -> bool {
    let mut all = String::new();
    for field in ["message", "error", "detail"] {
        if let Some(text) = reply.get(field).and_then(Value::as_str) {
            all.push_str(text);
            all.push(' ');
        }
    }
    all.to_lowercase().contains("incorrect service")
}

/// 提交一次的结果。
/// The outcome of one submission.
struct SubmitResult {
    /// 成功时用的是哪个服务编号。失败是空。
    /// Which service number worked. Empty on failure.
    service: Option<i64>,
    /// 服务器的原始回话。
    /// The server's reply as-is.
    reply: Value,
    /// 叠着查钥匙时顺手拿到的钥匙。
    /// A key picked up by the overlapping check, if any.
    key_found: Option<String>,
    /// 那个查钥匙的活儿还在跑，从这里取结果，不用另开一趟。
    /// The check is still running; take its result from here rather than starting a
    /// fresh one.
    key_channel: Option<mpsc::Receiver<Option<String>>>,
}

/// 提交一次，往前推一关。
///
/// 里面做了三件事：
/// 1. 等够 5 秒（从上次**发出**算起，不是从上次收到回话算起 —— 省一趟来回）
/// 2. 提交，被判太快就退避重试
/// 3. 同时叠着查钥匙（`overlap` 为真时）
///
/// Submit once, to move one checkpoint forward.
///
/// Three things happen inside:
/// 1. Wait out the 5 seconds (measured from when we last **sent**, not from when its
///    reply arrived — one round trip saved)
/// 2. Submit, backing off and retrying if told we are too fast
/// 3. Check for the key at the same time (when `overlap` is true)
fn submit_once(
    ticket: &str,
    service: Option<i64>,
    talk: bool,
    timer: &mut Timer,
    last_sent: &mut Option<Instant>,
    overlap: bool,
) -> SubmitResult {
    // 服务编号：优先用问出来的，不行就用兜底的。
    // Service numbers: prefer the one we were told, fall back otherwise.
    let mut to_try: Vec<i64> = Vec::new();
    if let Some(s) = service {
        to_try.push(s);
    }
    if !to_try.contains(&FALLBACK_SERVICE) {
        to_try.push(FALLBACK_SERVICE);
    }

    timer.start(steps::STEP);

    // ---- 等够间隔 ----
    // ---- Wait out the gap ----
    //
    // 这 5 秒是对面的硬规定，不能缩。但从"上次发出"算起而不是"上次收到"算起，
    // 就能把那趟来回的时间算在等待里，白省 0.2 秒。
    //
    // The 5 seconds is the far end's hard rule and cannot be shortened. But measuring
    // from "when we last sent" rather than "when its reply came back" counts that round
    // trip as part of the wait, saving 0.2 seconds for free.
    let need_to_wait = config::MIN_STEP_GAP + margin();
    if let Some(sent_at) = last_sent.as_ref() {
        let waited = sent_at.elapsed();
        if waited < need_to_wait {
            let more = need_to_wait - waited;
            say(
                talk,
                &format!(
                    "  [提交] 距上次发送{:.2}s，需补足{:.2}s(自适应余量{}ms)",
                    waited.as_secs_f64(),
                    more.as_secs_f64(),
                    margin().as_millis()
                ),
            );
            std::thread::sleep(more);
        }
    }

    let mut key_channel: Option<mpsc::Receiver<Option<String>>> = None;
    let mut checker_started = false;
    let found_key = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));

    for service_number in to_try {
        for attempt in 0..=config::STEP_THROTTLE_RETRIES {
            // ---- 叠着查钥匙 ----
            // ---- Start the overlapping check ----
            //
            // 提交发出去 50 毫秒后就开查，不等它回话。要是钥匙已经好了，这趟就白赚。
            //
            // Start checking 50ms after the submission goes out, without waiting for
            // its reply. If the key is already there, this trip comes free.
            if overlap && !checker_started {
                checker_started = true;

                let (sender, receiver) = mpsc::channel();
                key_channel = Some(receiver);

                let ticket = ticket.to_string();
                let shared = found_key.clone();

                std::thread::spawn(move || {
                    std::thread::sleep(config::POLL_OVERLAP_DELAY);

                    for _ in 0..config::POLL_MAX_ATTEMPTS {
                        let reply = auth::session_status(&ticket);
                        let body = reply.get("data").unwrap_or(&reply);
                        let key = body.get("key").and_then(Value::as_str).unwrap_or("");

                        if !key.is_empty() && key != "KEY_NOT_FOUND" {
                            *shared.lock().unwrap() = Some(key.to_string());
                            let _ = sender.send(Some(key.to_string()));
                            return;
                        }

                        std::thread::sleep(config::POLL_INTERVAL);
                    }

                    let _ = sender.send(None);
                });
            }

            // ---- 提交 ----
            // ---- Submit ----
            let sent_at = Instant::now();
            *last_sent = Some(sent_at);

            let reply = auth::submit(ticket, service_number);
            let took_ms = sent_at.elapsed().as_secs_f64() * 1000.0;

            // 花的时间明显偏长，说明里面重试过，真正最后一次发出比 sent_at 晚。
            // 那就改用"现在"当发出时刻，宁可下次多等一点也不能少等。
            //
            // Taking noticeably long means it retried inside, so the real last send was
            // later than sent_at. Use "now" instead — better to wait a little too long
            // next time than not long enough.
            if sent_at.elapsed() > Duration::from_millis(1000) {
                *last_sent = Some(Instant::now());
            }

            if reply.get("success").and_then(Value::as_bool) == Some(true) {
                timer.stop();
                note_good();
                say(talk, &format!("  [提交] 服务ID={} 成功({:.0}ms)", service_number, took_ms));

                let key_found = found_key.lock().unwrap().clone();

                return SubmitResult {
                    service: Some(service_number),
                    reply,
                    key_found,
                    key_channel,
                };
            }

            // 被判太快：余量加上去，歇一会儿再试。
            // Told we are too fast: put the margin up and rest before trying again.
            if says_too_fast(&reply) {
                note_too_fast();

                if attempt < config::STEP_THROTTLE_RETRIES {
                    say(
                        talk,
                        &format!(
                            "  [提交] 服务ID={} 触发频率限制，退避{}s后重试({}/{})",
                            service_number,
                            config::STEP_THROTTLE_SLEEP.as_secs(),
                            attempt + 1,
                            config::STEP_THROTTLE_RETRIES
                        ),
                    );
                    std::thread::sleep(config::STEP_THROTTLE_SLEEP);
                    continue;
                }
            }

            let why = reply.to_string();
            say(
                talk,
                &format!(
                    "  [提交] 服务ID={} 失败: {} ({:.0}ms)",
                    service_number,
                    &why[..why.len().min(200)],
                    took_ms
                ),
            );
            break;
        }
    }

    timer.stop();

    // 先把钥匙取出来存到变量里再拼结构，不然临时借用活不到函数返回。
    // Pull the key into a variable before building the result, otherwise the temporary
    // borrow does not live long enough to return.
    let key_found = found_key.lock().unwrap().clone();

    SubmitResult {
        service: None,
        reply: json!({ "success": false, "error": "全部服务ID均提交失败 / All service IDs failed" }),
        key_found,
        key_channel: None,
    }
}

// ---------------------------------------------------------------------------
// 主流程 / The main route
// ---------------------------------------------------------------------------

/// 走完一条链接的结果。
/// The outcome of working through one link.
pub struct Outcome {
    /// 拿到的钥匙。没拿到就是空。
    /// The key we got. Empty if we did not get one.
    pub key: Option<String>,
    /// 各步骤耗时。
    /// How long each step took.
    pub timer: Timer,
    /// 没拿到的话，是在哪一步停下的。看日志时有用。
    /// If we did not get one, where it stopped. Handy when reading logs.
    pub stopped_at: &'static str,
}
/// 从一条链接一路走到钥匙。
///
/// `talk` 为真会一路打印进度。`max_rounds` 是起始轮数上限，实际会按服务器说的关卡
/// 数往上调。
///
/// Work from one link all the way to a key.
///
/// With `talk` true it prints progress along the way. `max_rounds` is a starting cap,
/// and it gets raised to match however many checkpoints the server reports.
pub fn run(ticket: &str, talk: bool, max_rounds: usize) -> Outcome {
    let mut ticket = ticket.to_string();
    let mut timer = Timer::new();
    let mut rounds_allowed = max_rounds.max(1);
    let mut round = 0usize;
    let mut stopped_at = "已达轮数上限 / Round limit reached";
    let mut last_sent: Option<Instant> = None;

    while round < rounds_allowed {
        say(talk, &format!("  [第 {}/{} 轮]", round + 1, rounds_allowed));
        timer.start(steps::META);

        // ---- 三件事一起发 ----
        // ---- Three things at once ----
        //
        // 第一轮：问链接情况、查钥匙、提交 step 全部并行发出（提交不等 meta，
        // 服务编号先用兜底值）。万一这链接之前已经走完了，钥匙那趟直接给出结果。
        //
        // 后续轮：问情况和提交并行发出，提交内部再叠着查钥匙。
        //
        // First round: the link query, the key check and the step submission all go
        // out together (the submission does not wait for meta; it starts with the
        // fallback service number). If this link was already finished before, the
        // key check hands us the answer straight away.
        //
        // Later rounds: the query and the submission go out together, and the
        // submission itself overlaps the key check.
        let (info_sender, info_receiver) = mpsc::channel();
        let (key_sender, key_receiver) = mpsc::channel();

        let info_ticket = ticket.clone();
        let info_worker = std::thread::spawn(move || {
            let _ = info_sender.send(ask_about(&info_ticket, talk));
        });

        let key_worker = if round == 0 {
            let key_ticket = ticket.clone();
            Some(std::thread::spawn(move || {
                let _ = key_sender.send(check_key(&key_ticket, talk, None));
            }))
        } else {
            None
        };

        let mut result;

        if round == 0 {
            // ---- 第一轮：三件事同时发，谁也不等谁 ----
            // ---- First round: fire all three at once, none waits on another ----
            //
            // step 不等 meta：服务编号直接用兜底值（Delta=3）先发出去，
            // 和"问情况""查钥匙"并行 —— 省一趟串行往返。万一兜底服务编号被拒
            // （非 Delta 的链接），等 meta 回来拿它报的服务编号补一发就是。
            //
            // The step does not wait for meta: it goes straight out with the
            // fallback service number (Delta = 3), in parallel with the link query
            // and the key check — one serial round trip saved. Should the fallback
            // service number be refused (a non-Delta link), meta's number is used
            // for a follow-up attempt.
            result = submit_once(&ticket, None, talk, &mut timer, &mut last_sent, false);

            // 并行那趟"是否已完成"此刻早出结果了。链接之前走完过的话直接拿钥匙。
            // The parallel "already finished?" check has its answer by now. If the
            // link was completed before, take the key straight away.
            if let Some(worker) = key_worker {
                if let Ok(Some(key)) = key_receiver.recv_timeout(Duration::from_secs(6)) {
                    say(talk, "  [会话] 该链接此前已完成，直接返回既有密钥");
                    let _ = worker.join();
                    let _ = info_worker.join();

                    return Outcome {
                        key: Some(key),
                        timer,
                        stopped_at: "链接此前已完成 / Link was already completed",
                    };
                }
                let _ = worker.join();
            }

            // 收下 meta 的结果：死链终止、轮数放宽；服务编号被拒时准备补一发。
            // Take in the meta result: stop on a dead link, widen the round cap;
            // ready a follow-up should the service number have been refused.
            let mut retry_service: Option<i64> = None;

            match info_receiver.recv_timeout(Duration::from_secs(6)) {
                Ok(info) => {
                    let taken = take_in_info(info, talk, &mut rounds_allowed);

                    if let Some(why) = taken.dead {
                        timer.invalid_reason = Some(why.clone());
                        say(talk, &format!("  [终止] 链接已失效: {}", why));
                        let _ = info_worker.join();

                        return Outcome {
                            key: None,
                            timer,
                            stopped_at: "链接已失效 / Link is invalid",
                        };
                    }

                    if result.service.is_none() && says_wrong_service(&result.reply) {
                        retry_service = taken.service;
                    }
                }
                Err(_) => {
                    // 问不出来就当没这回事，提交已经按兜底服务编号发过了。
                    // Nothing came back, so never mind — the submission already went
                    // out with the fallback service number.
                    let _ = info_worker.join();
                }
            }

            timer.stop();

            // 兜底服务编号被拒，用 meta 报的那个补一发。
            // The fallback service number was refused, so retry with meta's.
            if let Some(svc) = retry_service {
                result = submit_once(&ticket, Some(svc), talk, &mut timer, &mut last_sent, false);
            }
        } else {
            // ---- 后续轮：先等 meta，再提交 ----
            // ---- Later rounds: wait for meta first, then submit ----
            //
            // 间隙等待反正要 5 秒，meta 那一趟完全被它盖住，等它拿准服务编号
            // 不吃亏。
            //
            // The gap wait is 5 seconds anyway, which completely covers the meta
            // trip, so waiting for it to get the service number right costs nothing.
            let mut service: Option<i64> = None;

            match info_receiver.recv_timeout(Duration::from_secs(6)) {
                Ok(info) => {
                    let taken = take_in_info(info, talk, &mut rounds_allowed);

                    if let Some(why) = taken.dead {
                        timer.invalid_reason = Some(why.clone());
                        say(talk, &format!("  [终止] 链接已失效: {}", why));
                        let _ = info_worker.join();

                        return Outcome {
                            key: None,
                            timer,
                            stopped_at: "链接已失效 / Link is invalid",
                        };
                    }

                    service = taken.service;
                }
                Err(_) => {
                    // 问不出来就往下走，用兜底服务编号。
                    // Nothing came back, so carry on with the fallback service number.
                    let _ = info_worker.join();
                }
            }

            timer.stop();

            // 后续轮叠着查钥匙：提交发出 50 毫秒后就开始查，不等回话。
            // Later rounds overlap the key check: it starts 50 milliseconds after
            // the submission goes out, without waiting for its reply.
            result = submit_once(&ticket, service, talk, &mut timer, &mut last_sent, true);
        }

        // 叠着查的那趟已经拿到钥匙了，直接结束。
        // The overlapping check already got the key, so we are done.
        if let Some(key) = result.key_found {
            say(talk, &format!("  [密钥] 已获取(并发轮询): {}", key));
            return Outcome {
                key: Some(key),
                timer,
                stopped_at: "成功 / Success",
            };
        }

        let Some(_worked) = result.service else {
            say(talk, &format!("  [跳过] 第{}轮提交全部失败", round + 1));
            stopped_at = "提交失败 / Submission failed";
            round += 1;
            continue;
        };

        // ---- 看服务器让我们去哪 ----
        // ---- See where the server is sending us ----
        let next_url = result
            .reply
            .get("data")
            .and_then(|d| d.get("url"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        if next_url.is_empty() {
            say(talk, "  [跳过] 响应中缺少跳转地址");
            stopped_at = "响应缺少地址 / Response missing address";
            round += 1;
            continue;
        }

        let shown = if next_url.len() > 80 {
            format!("{}...", &next_url[..80])
        } else {
            next_url.clone()
        };
        say(talk, &format!("  [跳转] {}", shown));

        // ---- 情况一：about:blank 表示走完了，去取钥匙 ----
        // ---- Case one: about:blank means we are through, so fetch the key ----
        if next_url == "about:blank" {
            // 那个叠着查的活儿还在跑，直接等它的结果，别另开一趟。
            // The overlapping check is still running, so wait on its result rather than
            // starting a fresh trip.
            if let Some(channel) = result.key_channel {
                timer.start(steps::POLL);
                let waited = channel.recv_timeout(
                    config::POLL_INTERVAL * config::POLL_MAX_ATTEMPTS as u32
                        + Duration::from_secs(2),
                );
                timer.stop();

                if let Ok(Some(key)) = waited {
                    say(talk, &format!("  [密钥] 已获取(并发轮询): {}", key));
                    return Outcome {
                        key: Some(key),
                        timer,
                        stopped_at: "成功 / Success",
                    };
                }
            }

            say(
                talk,
                &format!(
                    "  [轮询] 流程已完成，最多查询{}次，间隔{:.1}s",
                    config::POLL_MAX_ATTEMPTS,
                    config::POLL_INTERVAL.as_secs_f64()
                ),
            );

            if let Some(key) = wait_for_key(&ticket, talk, &mut timer) {
                return Outcome {
                    key: Some(key),
                    timer,
                    stopped_at: "成功 / Success",
                };
            }

            say(talk, &format!("  [跳过] 查询{}次后密钥仍未就绪", config::POLL_MAX_ATTEMPTS));
            stopped_at = "密钥未就绪 / Key never became ready";
            round += 1;
            continue;
        }

        // ---- 情况二：广告页链接，从里面解出下一段通行串 ----
        // ---- Case two: an advert page link, with the next pass string inside ----
        if let Some(callback) = auth::decode_callback(&next_url) {
            if let Some(next_ticket) = auth::ticket_from_callback(&callback) {
                // 太短的不像真的通行串，别当真。
                // Anything short is not a real pass string, so do not take it seriously.
                if next_ticket.len() > 50 {
                    say(
                        talk,
                        &format!(
                            "  [下一段] {}... (长度={})",
                            &next_ticket[..next_ticket.len().min(24)],
                            next_ticket.len()
                        ),
                    );
                    ticket = next_ticket;
                    round += 1;
                    continue;
                }
            }
        }

        // ---- 情况三：解不出下一段，最后碰碰运气查一下钥匙 ----
        // ---- Case three: nothing to decode, so try the key once just in case ----
        say(talk, "  [轮询] 无后续凭据，尝试查询密钥");
        if let Some(key) = check_key(&ticket, talk, Some(&mut timer)) {
            return Outcome {
                key: Some(key),
                timer,
                stopped_at: "成功 / Success",
            };
        }

        stopped_at = "无后续凭据且无密钥 / No next credential and no key";
        break;
    }

    // 出了循环再查一次，万一刚好好了。
    // One more check after the loop, in case it just became ready.
    if let Some(key) = check_key(&ticket, talk, Some(&mut timer)) {
        return Outcome {
            key: Some(key),
            timer,
            stopped_at: "成功 / Success",
        };
    }

    say(
        talk,
        &format!(
            "\n[失败] 未获取密钥 (原因: {}; 已执行 {}/{} 轮)",
            stopped_at, round, rounds_allowed
        ),
    );

    Outcome {
        key: None,
        timer,
        stopped_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 太快的话能认出来_recognises_being_told_too_fast() {
        let a = json!({ "message": "finishing checkpoints too fast" });
        assert!(says_too_fast(&a));

        let b = json!({ "error": "Please slow down" });
        assert!(says_too_fast(&b));

        let c = json!({ "detail": "TOO MANY requests" });
        assert!(says_too_fast(&c));

        let d = json!({ "message": "invalid payload." });
        assert!(!says_too_fast(&d), "链接废了不是太快 / a dead link is not going too fast");
    }

    #[test]
    fn 服务编号不对能认出来_recognises_a_wrong_service() {
        let a = json!({ "message": "incorrect service (advertiser) provided." });
        assert!(says_wrong_service(&a));

        let b = json!({ "message": "finishing checkpoints too fast" });
        assert!(
            !says_wrong_service(&b),
            "太快不是编号不对 / going too fast is not a wrong service number"
        );

        let c = json!({ "message": "invalid payload." });
        assert!(
            !says_wrong_service(&c),
            "链接废了不是编号不对 / a dead link is not a wrong service number"
        );
    }

    #[test]
    fn 收下情况会放宽轮数_take_in_info_widens_the_round_cap() {
        let info = LinkInfo {
            service: Some(3),
            checkpoints: Some(7),
            alive: true,
            dead_reason: None,
        };

        let mut rounds = 3usize;
        let taken = take_in_info(info, false, &mut rounds);

        assert!(taken.dead.is_none());
        assert_eq!(taken.service, Some(3));
        assert_eq!(rounds, 8, "7 个检查点应该放宽到 8 轮 / 7 checkpoints should widen to 8 rounds");
    }

    #[test]
    fn 死链会带出原因_take_in_info_carries_the_reason() {
        let info = LinkInfo {
            service: None,
            checkpoints: None,
            alive: false,
            dead_reason: Some("expired".into()),
        };

        let mut rounds = 3usize;
        let taken = take_in_info(info, false, &mut rounds);

        assert_eq!(taken.dead.as_deref(), Some("expired"));
        assert_eq!(rounds, 3, "死链不该动轮数 / a dead link should not touch the round cap");
    }

    #[test]
    fn 余量会自己往下减_margin_shrinks_by_itself() {
        // 从起始值开始，连续顺利应该让余量变小。
        // Starting from the initial value, good runs should shrink the margin.
        MARGIN_MS.store(config::GAP_MARGIN_START.as_millis() as u64, Ordering::Relaxed);
        GOOD_RUNS.store(0, Ordering::Relaxed);

        let before = margin();
        for _ in 0..config::CLEAN_STEPS_TO_RELAX {
            note_good();
        }
        let after = margin();

        assert!(after < before, "顺利了应该减少余量 / good runs should shrink the margin");
    }

    #[test]
    fn 被拒之后余量马上加上去_margin_jumps_after_a_refusal() {
        MARGIN_MS.store(config::GAP_MARGIN_MIN.as_millis() as u64, Ordering::Relaxed);

        let before = margin();
        note_too_fast();
        let after = margin();

        assert!(after > before, "被拒应该马上加余量 / a refusal should raise the margin at once");
    }

    #[test]
    fn 余量不会越界_margin_stays_within_bounds() {
        // 拼命减也不能低于下限。
        // Shrinking hard should still not go below the floor.
        MARGIN_MS.store(config::GAP_MARGIN_MIN.as_millis() as u64, Ordering::Relaxed);
        GOOD_RUNS.store(0, Ordering::Relaxed);
        for _ in 0..100 {
            note_good();
        }
        assert!(
            margin() >= config::GAP_MARGIN_MIN,
            "不该低于下限 / should not go below the floor"
        );

        // 拼命加也不能超过上限。
        // Growing hard should still not exceed the ceiling.
        for _ in 0..100 {
            note_too_fast();
        }
        assert!(
            margin() <= config::GAP_MARGIN_MAX,
            "不该超过上限 / should not exceed the ceiling"
        );
    }
}
