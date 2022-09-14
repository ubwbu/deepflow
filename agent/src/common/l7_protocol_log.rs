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

use enum_dispatch::enum_dispatch;
use public::l7_protocol::L7Protocol;
use serde::Serialize;

use super::ebpf::EbpfType;
use super::flow::PacketDirection;
use super::MetaPacket;

use crate::common::lookup_key::LookupKey;
use crate::config::handler::LogParserAccess;
use crate::flow_generator::protocol_logs::pb_adapter::L7ProtocolSendLog;
use crate::flow_generator::protocol_logs::{
    DnsLog, DubboLog, HttpLog, KafkaLog, MqttLog, MysqlLog, RedisLog,
};
use crate::flow_generator::AppProtoHead;
use crate::flow_generator::Error;
use crate::flow_generator::Result;

#[macro_export]
macro_rules! __log_info_merge {
    ($self:ident,$log_type:ident,$other:ident) => {
        if let L7ProtocolLog::$log_type(__other) = $other {
            if __other.start_time < $self.start_time {
                $self.start_time = __other.start_time;
            }
            if __other.end_time > $self.end_time {
                $self.end_time = __other.end_time;
            }
            $self.info.merge(__other.info);
        }
        return Ok(());
    };
}

// 忽略非原始数据类型
#[macro_export]
macro_rules! ignore_non_raw_protocol {
    ($parse_param:ident) => {
        if !$parse_param.ebpf_type.is_raw_protocol() {
            return false;
        }
    };
}

/*
 所有协议都需要实现这个接口.
 其中,check 用于MetaPacket判断应用层协议, parse用于解析具体协议.
 更具体就是遍历ALL_PROTOCOL的协议,用check判断协议,再用parse解析整个payload, 实现的结构应该在parse的时候记录关键字段.
 最后发送到server之前, 调用into() 转成通用结构L7ProtocolLogData.

 ebpf处理过程为:


                                   payload:&[u8]
                                         |
                                         |
                                    MetaPacket
                                         |
                                         |
                                         |
                                      check()
                                         |
                                         |
                           traversal all implement protocol
                                         |
                                         |
                                         |
                            L7ProtocolLog::check_payload()
                                  |           |
                                  |           |
                                  v           v
                     <-----------true       false-------->set protocol as unknown, then ignore the packet.
                    |
                    |
                    v
       L7protocolLog::parse_payload()
                    |
                    |
         |<---------v----------->Vec<L7ProtocolLog>-------->
         |                                                 |
         v                                                 |
 raise err, ignore the packet                              v
                                                      for each log
                                                           |
                                                           |
                                                           v
                           find the req/resp in SessionAggr, only find 2 solt(include current solt)
                                                           |
                                                           |
                           | <------found req/resp <------ v------> not found ------->save in current slot , wait req/resp
                           |
                           |
                           v
             L7protocolLog::merge_log(req/resp)   (merge req and resp to session)
                           |
                           |
                           v
               ! L7protocolLog::skip_send()
                           |
                           v
                    send to server


about SessionAggr:

    [
        hashmap< key = u64, value = L7protocolLog >, (represent 60s)
        hashmap< key = u64, value = L7protocolLog >, (represent 60s)
        ....
    ]

    it is time slot array(default length 16) + hashmap struct, every time slot repersent 60s time.

    key is u64 : | flow_id hight 8 bit | flow_id low 24 bit | proto 8 bit | session low 24 bit |

    flow_id: from ebpf socket_id, distinguish the socket fd.
    proto:   protocol number, such as: tcp=6 udp=17
    session: depend on the protocol, for example http2:stream_id,  dns:transaction_id.



about check():
    check() will travsal all protocol from get_all_protocol() to determine what protocol belong to the payload.
    first, check the bitmap(config by server,describe follow) is ignore the protocol ? then call L7ProtocolLog::check_payload() to check.

about parse_payload()
    it use same struct in L7ProtocolLog::check_payload().

about bitmap:
    u128, every bit repersent the protocol shoud check or not(1 indicate check, 0 for ignore), the number of protocol as follow:

    const L7_PROTOCOL_UNKNOWN: u8 = 0;
    const L7_PROTOCOL_HTTP1: u8 = 20;
    const L7_PROTOCOL_HTTP2: u8 = 21;
    const L7_PROTOCOL_HTTP1_TLS: u8 = 22;
    const L7_PROTOCOL_HTTP2_TLS: u8 = 23;
    const L7_PROTOCOL_DUBBO: u8 = 40;
    const L7_PROTOCOL_MYSQL: u8 = 60;
    const L7_PROTOCOL_REDIS: u8 = 80;
    const L7_PROTOCOL_KAFKA: u8 = 100;
    const L7_PROTOCOL_MQTT: u8 = 101;
    const L7_PROTOCOL_DNS: u8 = 120;



 TODO: cbpf 处理过程
 hint: check 和 parse 是同一个结构, check可以把解析结果保存下来,避免重复解析.

*/

pub struct EbpfParam {
    pub is_tls: bool,
    // 目前仅 http2 uprobe 有意义
    pub is_req_end: bool,
    pub is_resp_end: bool,
}

pub struct ParseParam {
    pub direction: PacketDirection,
    pub ebpf_type: EbpfType,
    // ebpf_type 不为 EBPF_TYPE_NONE 会有值
    pub ebpf_param: Option<EbpfParam>,
    pub time: u64,
}

impl ParseParam {
    pub fn from(packet: &MetaPacket) -> Self {
        let mut param = Self {
            direction: packet.direction,
            ebpf_type: packet.ebpf_type,
            ebpf_param: None,
            time: packet.start_time.as_micros() as u64,
        };
        if packet.ebpf_type != EbpfType::None {
            let is_tls = match packet.ebpf_type {
                EbpfType::TlsUprobe => true,
                _ => match packet.l7_protocol_from_ebpf {
                    L7Protocol::Http1TLS | L7Protocol::Http2TLS => true,
                    _ => false,
                },
            };
            param.ebpf_param = Some(EbpfParam {
                is_tls: is_tls,
                is_req_end: packet.is_request_end,
                is_resp_end: packet.is_response_end,
            });
        }
        return param;
    }
}

#[enum_dispatch(L7ProtocolLog)]
pub trait L7ProtocolLogInterface {
    // 个别协议一个连接可能有子流, 这里需要返回流标识, 例如http2的stream id
    fn session_id(&self) -> Option<u32>;
    // 协议字段合并
    // enum_dispatch 不能使用&Self 参数, 这里只能使用&L7Protocol.
    fn merge_log(&mut self, other: L7ProtocolLog) -> Result<()>;
    // 协议判断
    // direction 并非所有协议都准确.
    fn check_payload(&mut self, payload: &[u8], lookup_key: &LookupKey, param: ParseParam) -> bool;
    // 协议解析
    fn parse_payload(
        self,
        payload: &[u8],
        lookup_key: &LookupKey,
        param: ParseParam,
    ) -> Result<Vec<L7ProtocolLog>>;
    // 返回协议号和协议名称, 由于的bitmap使用u128,所以协议号不能超过128.
    // 其中 src/common/flow.rs 里面的 pub const L7_PROTOCOL_xxx 是内部保留的协议号.
    fn protocol(&self) -> (L7Protocol, &str);
    fn app_proto_head(&self) -> Option<AppProtoHead>;
    fn is_tls(&self) -> bool;
    fn skip_send(&self) -> bool;

    // 是否需要进一步合并, 目前只有在ebpf有意义, 内置协议也只有 EBPF_TYPE_GO_HTTP2_UPROBE 会用到.
    // 除非确实需要多次log合并,否则应该一律返回false
    fn need_merge(&self) -> bool {
        return false;
    }
    // 对于需要多次merge的情况下,判断流是否已经结束,只有在need_merge->true的情况下有用
    // 返回 req_end,resp_end
    fn is_req_resp_end(&self) -> (bool, bool) {
        return (false, false);
    }
    // 仅http和dubbo协议会有log_parser_config，其他协议可以忽略。
    fn set_parse_config(&mut self, _log_parser_config: &LogParserAccess) {}
    // l4是tcp是是否解析，用于快速过滤协议
    fn parse_on_tcp(&self) -> bool {
        return true;
    }
    // l4是udp是是否解析，用于快速过滤协议
    fn parse_on_udp(&self) -> bool {
        return true;
    }
    fn into_l7_protocol_send_log(self) -> L7ProtocolSendLog;

    fn get_default(&self) -> L7ProtocolLog;
}

#[derive(Debug, Clone, Serialize)]
#[enum_dispatch]
pub enum L7ProtocolLog {
    L7ProtocolUnknown(L7ProtocolLogUnknown),
    DnsLog(DnsLog),
    HttpLog(HttpLog),
    MysqlLog(MysqlLog),
    RedisLog(RedisLog),
    DubboLog(DubboLog),
    KafkaLog(KafkaLog),
    MqttLog(MqttLog),
    // add new protocol here
}

impl Into<L7ProtocolSendLog> for L7ProtocolLog {
    fn into(self) -> L7ProtocolSendLog {
        return self.into_l7_protocol_send_log();
    }
}

impl L7ProtocolLog {
    pub fn is_skip_parse(&self, bitmap: u128) -> bool {
        return bitmap & (1 << (self.protocol().0 as u8)) == 0;
    }

    pub fn set_bitmap_skip_parse(&self, bitmap: &mut u128) {
        *bitmap &= !(1 << (self.protocol().0 as u8));
    }

    pub fn is_session_end(&self) -> bool {
        let (req_end, resp_end) = self.is_req_resp_end();
        return req_end && resp_end;
    }
}
#[derive(Clone, Copy, Default, Debug, Serialize)]
pub struct L7ProtocolLogUnknown {}
impl L7ProtocolLogInterface for L7ProtocolLogUnknown {
    fn session_id(&self) -> Option<u32> {
        return None;
    }

    fn merge_log(&mut self, _other: L7ProtocolLog) -> Result<()> {
        return Ok(());
    }

    fn check_payload(&mut self, _payload: &[u8], _lk: &LookupKey, _param: ParseParam) -> bool {
        return true;
    }

    fn parse_payload(
        self,
        _payload: &[u8],
        _lk: &LookupKey,
        _param: ParseParam,
    ) -> Result<Vec<L7ProtocolLog>> {
        return Err(Error::L7ProtocolCheckLimit);
    }

    fn protocol(&self) -> (L7Protocol, &str) {
        return (L7Protocol::Unknown, "UNKNOWN");
    }

    fn app_proto_head(&self) -> Option<AppProtoHead> {
        return None;
    }

    fn into_l7_protocol_send_log(self) -> L7ProtocolSendLog {
        return L7ProtocolSendLog::default();
    }

    fn is_tls(&self) -> bool {
        return false;
    }
    // 是否上报数据到server
    fn skip_send(&self) -> bool {
        return true;
    }

    fn get_default(&self) -> L7ProtocolLog {
        return L7ProtocolLog::L7ProtocolUnknown(*self);
    }
}

// 内部实现的协议
// log的具体结构和实现在 src/flow_generator/protocol_logs/** 下
pub fn get_all_protocol() -> Vec<L7ProtocolLog> {
    let mut all_proto = vec![
        L7ProtocolLog::DnsLog(DnsLog::default()),
        L7ProtocolLog::HttpLog(HttpLog::new_v1()),
        L7ProtocolLog::HttpLog(HttpLog::new_v2()),
        L7ProtocolLog::KafkaLog(KafkaLog::default()),
        L7ProtocolLog::MysqlLog(MysqlLog::default()),
        L7ProtocolLog::RedisLog(RedisLog::default()),
        L7ProtocolLog::DubboLog(DubboLog::default()),
        L7ProtocolLog::MqttLog(MqttLog::default()),
    ];
    all_proto.extend(get_ext_protocol());

    return all_proto;
}

// 所有拓展协议，实际上是perf没有实现的协议。 由于perf没有抽象出来，需要区分出来
// 后面perf抽象出来后会去掉，只留下get_all_protocol
pub fn get_ext_protocol() -> Vec<L7ProtocolLog> {
    let all_proto = vec![
        // add protocol log implement here
    ];
    return all_proto;
}