use klyndb_driver_api::{Error, KeyCommandInfo, Result};

pub(crate) fn parse(text: &str) -> Result<(Vec<String>, KeyCommandInfo)> {
    if text.len() > 256 * 1024 {
        return Err(Error::new("Redis commands are limited to 256 KiB"));
    }
    let mut arguments: Vec<String> = serde_json::from_str(text).map_err(|_| {
        Error::new("Enter a JSON array of command arguments, such as [\"GET\", \"cache:key\"]")
    })?;
    if arguments.is_empty() || arguments.len() > 1000 {
        return Err(Error::new("Enter between 1 and 1,000 command arguments"));
    }
    let command = arguments[0].to_ascii_uppercase();
    let writes = match command.as_str() {
        "PING" | "ECHO" | "TIME" | "DBSIZE" | "GET" | "MGET" | "GETRANGE" | "STRLEN" | "TYPE"
        | "TTL" | "PTTL" | "EXISTS" | "SCAN" | "HGET" | "HMGET" | "HGETALL" | "HSCAN" | "HLEN"
        | "HKEYS" | "HVALS" | "HEXISTS" | "LLEN" | "LRANGE" | "LINDEX" | "SCARD" | "SMEMBERS"
        | "SSCAN" | "SISMEMBER" | "SRANDMEMBER" | "ZCARD" | "ZRANGE" | "ZREVRANGE" | "ZSCORE"
        | "ZSCAN" | "ZCOUNT" | "XRANGE" | "XREVRANGE" | "XLEN" => false,
        "SET" | "SETNX" | "SETEX" | "PSETEX" | "MSET" | "MSETNX" | "DEL" | "UNLINK" | "EXPIRE"
        | "PEXPIRE" | "EXPIREAT" | "PEXPIREAT" | "PERSIST" | "RENAME" | "RENAMENX" | "INCR"
        | "INCRBY" | "INCRBYFLOAT" | "DECR" | "DECRBY" | "APPEND" | "SETRANGE" | "HSET"
        | "HSETNX" | "HDEL" | "HINCRBY" | "HINCRBYFLOAT" | "LPUSH" | "RPUSH" | "LPUSHX"
        | "RPUSHX" | "LSET" | "LINSERT" | "LREM" | "LTRIM" | "LPOP" | "RPOP" | "RPOPLPUSH"
        | "LMOVE" | "SADD" | "SREM" | "SPOP" | "SMOVE" | "ZADD" | "ZREM" | "ZINCRBY"
        | "ZREMRANGEBYSCORE" | "ZREMRANGEBYRANK" | "ZPOPMAX" | "ZPOPMIN" | "XADD" | "XDEL"
        | "XTRIM" => true,
        _ => {
            return Err(Error::new(
                "This command is not supported. Use key data commands; session, transaction, blocking, scripting and administrative commands are unavailable.",
            ));
        }
    };
    arguments[0] = command.clone();
    let count = arguments.len() - 1;
    Ok((
        arguments,
        KeyCommandInfo {
            command,
            writes,
            arguments: count,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_arguments_and_readonly_classification() {
        let (args, info) = parse(r#"["gEt","a key\nwith 'quotes'\u0000"]"#).unwrap();
        assert_eq!(args, ["GET", "a key\nwith 'quotes'\0"]);
        assert!(!info.writes);
        assert!(parse(r#"["SET","key","value","GET"]"#).unwrap().1.writes);
        for command in [
            "AUTH",
            "HELLO",
            "SELECT",
            "MULTI",
            "EXEC",
            "WATCH",
            "EVAL",
            "FCALL",
            "CONFIG",
            "ACL",
            "KEYS",
            "FLUSHDB",
            "BLPOP",
            "SUBSCRIBE",
            "GEORADIUS",
            "GET\r\nSET",
        ] {
            assert!(parse(&serde_json::json!([command]).to_string()).is_err());
        }
        for bad in [
            "GET key",
            "[]",
            "[1]",
            "[\"GET\",null]",
            "[\"GET\"];[\"DEL\",\"key\"]",
        ] {
            assert!(parse(bad).is_err());
        }
        assert!(parse(&serde_json::json!(vec!["GET"; 1001]).to_string()).is_err());
        assert!(parse(&" ".repeat(256 * 1024 + 1)).is_err());
    }
}
