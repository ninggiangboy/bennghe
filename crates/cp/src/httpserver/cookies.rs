// SPDX-License-Identifier: AGPL-3.0-only
//! Session cookie handling. The `__Host-` prefix makes browsers insist on `Secure`, `Path=/` and
//! no `Domain`, so a sibling subdomain (a preview app, say) cannot plant or read it.
use axum::http::HeaderMap;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::Duration;

use crate::config::Config;

pub fn name(config: &Config) -> &'static str {
    if config.insecure_cookies { "bn_sid" } else { "__Host-bn_sid" }
}

pub fn build(config: &Config, token: String, ttl: Duration) -> Cookie<'static> {
    Cookie::build((name(config), token))
        .http_only(true)
        .secure(!config.insecure_cookies)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time_duration(ttl))
        .build()
}

pub fn clear(config: &Config) -> Cookie<'static> {
    let mut c = build(config, String::new(), Duration::zero());
    c.make_removal();
    c
}

pub fn read(config: &Config, headers: &HeaderMap) -> Option<String> {
    CookieJar::from_headers(headers).get(name(config)).map(|c| c.value().to_owned())
}

fn time_duration(d: Duration) -> time::Duration {
    time::Duration::seconds(d.num_seconds())
}
