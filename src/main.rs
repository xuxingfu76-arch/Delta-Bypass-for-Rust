//! Delta 绕过器 —— 命令行入口
//! Delta bypass — command line entry point
//!
//! 三种用法：
//!
//! ```text
//! delta-bypass "<链接>"              绕过一条
//! delta-bypass --generate 3          生成 3 条测试链接并绕过
//! delta-bypass --serve               开成网页接口
//! ```
//!
//! Three ways to use it:
//!
//! ```text
//! delta-bypass "<link>"              bypass one
//! delta-bypass --generate 3          make 3 test links and bypass them
//! delta-bypass --serve               run as a web interface
//! ```

use std::time::{Duration, Instant};

use clap::{CommandFactory, Parser};

use delta_bypass::{api, auth, chain, config, link, net, platform, timing};

/// 命令行参数。
/// Command line options.
#[derive(Parser)]
#[command(
    name = "delta-bypass",
    version,
    about = "Delta 钥匙绕过器 / Delta key bypass",
    long_about = "自动处理关卡，从链接拿到钥匙。\n\
                  Handles checkpoints automatically, turning a link into a key."
)]
struct Options {
    /// 要绕过的东西：链接、通行串，或者存了这些的文件
    /// What to bypass: a link, a pass string, or a file holding them
    target: Option<String>,

    /// 生成几条测试链接来试
    /// How many test links to make and try
    #[arg(short = 'g', long = "generate", default_value_t = 0)]
    generate: usize,

    /// 只报结果，不报过程
    /// Report the result only, not the progress
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,

    /// 最多走几轮（实际会按服务器说的关卡数往上调）
    /// Round cap, raised to match however many checkpoints the server reports
    #[arg(long = "max-rounds", default_value_t = config::DEFAULT_MAX_ROUNDS)]
    max_rounds: usize,

    /// 只生成链接，不绕过
    /// Only make links, do not bypass
    #[arg(long = "no-auto")]
    no_auto: bool,

    /// 开成网页接口
    /// Run as a web interface
    #[arg(long = "serve")]
    serve: bool,

    /// 网页接口监听哪个端口
    /// Which port the web interface listens on
    #[arg(short = 'p', long = "port", default_value_t = config::DEFAULT_PORT)]
    port: u16,

    /// 网页接口监听哪个地址（只想本机能用就填 127.0.0.1）
    /// Which address it listens on (use 127.0.0.1 for local access only)
    #[arg(long = "host", default_value_t = config::DEFAULT_HOST.to_string())]
    host: String,
}

fn main() {
    // Windows 上把工作目录切到 exe 旁边，免得钥匙文件写到莫名其妙的地方去。
    // Linux 上不动，服务通常自己指定了工作目录。
    //
    // On Windows, move the working directory next to the exe, so the keys file does not
    // land somewhere unexpected. Left alone on Linux, where the service usually sets its
    // own.
    platform::use_program_folder();

    let options = Options::parse();

    if options.serve {
        api::serve(&options.host, options.port);
        return;
    }

    bypass_links(&options);
}

// ---------------------------------------------------------------------------
// 正事：绕过链接 / The real job: bypassing links
// ---------------------------------------------------------------------------

/// 绕过一条或几条链接。
/// Bypass one or several links.
fn bypass_links(options: &Options) {
    let talk = !options.quiet;
    let overall_start = Instant::now();

    // 一边开连接、一边盯版本，一边去要链接 —— 不用等对方。
    // Open connections, watch the client version, and ask for links — none waits on
    // the others.
    let warming = std::thread::spawn(net::warm_up_all);
    delta_bypass::clientver::start_background();

    // 先把要绕过的通行串凑齐。
    // Gather the pass strings to work through.
    let tickets: Vec<String> = if options.generate > 0 {
        if talk {
            println!("[准备] 正在生成 {} 条测试链接 / Generating {} test links", options.generate, options.generate);
        }

        let urls = link::create_many(
            options.generate,
            link::SERVICE_ANDROID,
            Duration::from_millis(300),
        );

        if talk {
            println!("[准备] 已获取 {} 条 / Obtained {}", urls.len(), urls.len());
        }

        urls.iter().map(|u| auth::extract_ticket(u)).collect()
    } else if let Some(given) = &options.target {
        // 命令行参数才允许从文件读凭据（一行一条那种）。
        // Only a command line argument may read credentials from a file (one per line).
        vec![auth::extract_ticket_from_arg(given)]
    } else {
        let _ = Options::command().print_help();
        std::process::exit(1);
    };

    // 只要链接不绕过。
    // Only make links, without bypassing.
    if options.no_auto {
        let _ = warming.join();
        for t in tickets {
            println!("https://auth.platorelay.com/a?d={}", t);
        }
        return;
    }

    let _ = warming.join();

    // ---- 一条一条绕 ----
    // ---- Work through them one at a time ----
    let mut results: Vec<(Option<String>, timing::Timer, f64)> = Vec::new();

    for (index, ticket) in tickets.iter().enumerate() {
        let one_start = Instant::now();
        let outcome = chain::run(ticket, talk, options.max_rounds);
        let wall_secs = one_start.elapsed().as_secs_f64();

        if let Some(key) = &outcome.key {
            println!("\n{}", "=".repeat(60));
            println!("[成功] 第{}条密钥 / Key #{}: {}", index + 1, index + 1, key);
            println!("[成功] 耗时 {:.1}s / Elapsed {:.1}s", wall_secs, wall_secs);
            println!("[成功] 阶段明细 / Stage breakdown:");
            println!("{}", outcome.timer.summary());
            println!("{}", "=".repeat(60));
        } else if talk {
            println!("\n[失败] 第{}条未获取密钥 / No key for #{}", index + 1, index + 1);
            println!("[失败] 终止于: {}", outcome.stopped_at);
            if !outcome.timer.is_empty() {
                println!("[失败] 耗时 {:.1}s / Elapsed {:.1}s", wall_secs, wall_secs);
                println!("[失败] 阶段明细 / Stage breakdown:");
                println!("{}", outcome.timer.summary());
            }
        }

        results.push((outcome.key, outcome.timer, wall_secs));
    }

    // ---- 总结 ----
    // ---- Wrap up ----
    let total_secs = overall_start.elapsed().as_secs_f64();
    let succeeded = results.iter().filter(|r| r.0.is_some()).count();

    // 把所有条的各步骤耗时合起来，看整体时间花在哪。
    // Add up every link's step timings, to see where the time went overall.
    let mut combined = timing::Timer::new();
    for (_, one, _) in &results {
        for (step, secs) in one.iter() {
            combined.add(step, *secs);
        }
    }

    println!("\n{}", "=".repeat(60));
    println!("[汇总] 成功 {}/{} 条 / {} of {} succeeded", succeeded, tickets.len(), succeeded, tickets.len());
    println!("[汇总] 总耗时 {:.1}s / {:.1}s total", total_secs, total_secs);

    if !tickets.is_empty() {
        println!(
            "[汇总] 平均每条 {:.1}s / {:.1}s per link on average",
            total_secs / tickets.len() as f64,
            total_secs / tickets.len() as f64
        );
    }

    if !combined.is_empty() {
        println!("[汇总] 阶段合计 / Stage totals:");
        println!("{}", combined.summary());
    }

    println!("{}", "=".repeat(60));
}
