//! 网络连接：建好放着反复用
//! Network connections: build once, keep using
//!
//! 为什么不每次新建一条连接？因为握手很贵。走已经开好的连接发一次请求，比新开
//! 一条快一倍 —— 差的时间全花在握手和"慢启动"上（新连接一开始不敢快传，要试探
//! 着加速）。
//!
//! 所以整个程序共用一份连接，闲着也留着不关。
//!
//! Why not open a fresh connection each time? Because handshakes are expensive. A
//! request over an already-open connection is about twice as fast as over a new one
//! — all the difference is handshaking and "slow start" (a new connection does not
//! dare send fast at first and has to feel its way up).
//!
//! So the whole program shares one connection, kept open even while idle.
//!
//! 注：上游已取消图形验证码，连向验证码服务器的连接已随之移除，这里只剩登录
//! 服务器的连接预热。
//!
//! Note: the far end dropped the picture captcha, so the connection to the puzzle
//! server went with it; only the login-server warm-up remains.

use crate::auth;

/// 两个服务器的连接一起预热。
///
/// 两边同时开，别一个等一个。
///
/// Warm up both servers' connections.
///
/// Both at once, rather than one waiting for the other.
pub fn warm_up_all() {
    let _ = std::thread::spawn(auth::warm_up);
}
