/*
 * Copyright (c) 2022 Yunshan Networks
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("parse mac address failed: {0}")]
    ParseMacFailed(String),
    #[error("neighbor lookup failed from: {0}")]
    NeighborLookup(String),
    #[error("link not found: {0}")]
    LinkNotFound(String),
    #[error("link not found index: {0}")]
    LinkNotFoundIndex(u32),
    #[error("link regex invalid")]
    LinkRegexInvalid(#[from] regex::Error),
    #[error("netlink error")]
    NetLinkError(#[from] neli::err::NlError),
    #[error("IO error")]
    IoError(#[from] std::io::Error),
    #[error("no route to host: {0}")]
    NoRouteToHost(String),
    #[error("Windows related error:{0}")]
    Windows(String),
    #[error("{0}")]
    LinkIdxNotFoundByIP(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
