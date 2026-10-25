//! 策略与模块的 JSON Schema（Draft 2020-12）。

use serde_json::{Value, json};

pub struct SchemaFailure {
    pub pointer: String,
    pub message: String,
}

pub fn validate_module(module: &Value) -> Vec<SchemaFailure> {
    validate_with("模块", &module_schema(), module)
}

pub fn validate_system(system: &Value) -> Vec<SchemaFailure> {
    validate_with("系统", &system_schema(), system)
}

pub fn validate_named(schema: &Value, instance: &Value) -> Vec<SchemaFailure> {
    validate_with("spec", schema, instance)
}

fn validate_with(what: &str, schema: &Value, instance: &Value) -> Vec<SchemaFailure> {
    let validator = match jsonschema::validator_for(schema) {
        Ok(validator) => validator,
        Err(error) => {
            return vec![SchemaFailure {
                pointer: String::new(),
                message: format!("{what} schema 无法编译: {error}"),
            }];
        }
    };
    validator
        .iter_errors(instance)
        .map(|error| SchemaFailure {
            pointer: error.instance_path().to_string(),
            message: format!("{what} 不符合 schema: {error}"),
        })
        .collect()
}

fn duration() -> Value {
    json!({"type": "string", "pattern": "^[1-9][0-9]*(ms|s|m|h|d)$"})
}

fn duration_alt(extra: &str) -> Value {
    json!({"type": "string", "pattern": format!("^([1-9][0-9]*(ms|s|m|h|d)|{extra})$")})
}

fn size() -> Value {
    json!({"type": "string", "pattern": "^[1-9][0-9]*(B|KiB|MiB|GiB)$"})
}

fn retention() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {"tasks": duration()}
    })
}

fn policy_body() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "preset": {"type": "string", "minLength": 1},
            "locks": {"type": "array", "items": {"type": "string", "minLength": 1}},
            "raw": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "store": {"enum": ["all", "on-failure", "sampled", "none"]},
                    "sample_rate": {"type": "number", "minimum": 0, "maximum": 1},
                    "retain": duration_alt("forever"),
                    "max_part_size": size(),
                    "redact_headers": {"type": "array", "items": {"type": "string"}}
                }
            },
            "media": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "mode": {"enum": ["reference", "download"]},
                    "kinds": {"type": "array", "items": {"enum": ["image", "audio", "video", "file"]}},
                    "max_size": size()
                }
            },
            "versions": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "facet_keep": {
                        "type": "object",
                        "additionalProperties": {
                            "oneOf": [
                                {"type": "integer", "minimum": 1},
                                {"const": "all"}
                            ]
                        }
                    },
                    "snapshot_retain": duration(),
                    "audit_tag": {"enum": ["none", "daily", "weekly"]}
                }
            },
            "commit": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "flush_interval": duration(),
                    "flush_max_rows": {"type": "integer", "minimum": 1}
                }
            },
            "state": {
                "type": "object",
                "additionalProperties": false,
                "properties": {"snapshot_interval": duration()}
            },
            "robots": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "mode": {"enum": ["obey", "obey-with-exemptions", "ignore"]},
                    "exemptions": {"type": "array", "items": {"type": "string", "minLength": 1}},
                    "cache_ttl": duration(),
                    "on_unreachable": {"enum": ["rfc9309", "allow", "disallow"]},
                    "crawl_delay": {"type": "string", "pattern": "^(honor|ignore|cap:[1-9][0-9]*(ms|s|m|h|d))$"},
                    "exempt_as": {"type": "string", "minLength": 1},
                    "robots_reason": {"type": "string", "minLength": 1}
                }
            },
            "render": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "max_level": {"enum": ["L0", "L1", "L2", "L3", "L4"]},
                    "fallback_engine": {"enum": ["none", "chromium"]},
                    "downgrade_probe": duration_alt("never")
                }
            },
            "capture": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "screenshot": {"enum": ["never", "on-failure", "on-render", "always"]},
                    "network": {"enum": ["off", "trace", "matched-body", "all-bodies"]},
                    "renditions": {"type": "array", "items": {"enum": ["markdown", "semantic_tree_text"]}}
                }
            },
            "egress": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "private_networks": {"enum": ["deny", "allow"]},
                    "proxy": {"type": "string", "pattern": "^(system|off|https?://[^\\s]+)$"},
                    "identity": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "user_agent": {"type": "string", "minLength": 1},
                            "contact": {"type": "string"},
                            "web_bot_auth": {"enum": ["off", "on"]}
                        }
                    }
                }
            },
            "paid": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "unknown_receipt": {"enum": ["retry-once", "manual"]}
                }
            },
            "reprocess": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "on_adapter_change": {"type": "string", "pattern": "^(none|latest|window:[1-9][0-9]*(ms|s|m|h|d))$"}
                }
            },
            "identity": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "merge_primary": {"enum": ["earliest", "external-id-first", "manual"]},
                    "strip_params": {"type": "array", "items": {"type": "string"}},
                    "fragment": {"enum": ["drop", "keep"]}
                }
            },
            "dedupe": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "near_duplicate": {"enum": ["off", "simhash", "embedding"]}
                }
            },
            "backfill": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "initial": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "limit": {"type": "integer", "minimum": 0},
                            "max_age": duration()
                        }
                    }
                }
            },
            "classify": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "detector": {"type": "string", "pattern": "^(basic|off|plugin:[A-Za-z0-9][A-Za-z0-9._:-]*)$"}
                }
            },
            "retention": retention(),
            "health": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "expected_period": duration(),
                    "yield_drop_ratio": {"type": "number", "minimum": 0, "maximum": 1},
                    "failure_threshold": {"type": "integer", "minimum": 1}
                }
            }
        }
    })
}

fn limits() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["timeout", "concurrency", "retry"],
        "properties": {
            "timeout": duration(),
            "concurrency": {"type": "integer", "minimum": 1},
            "retry": {
                "type": "object",
                "additionalProperties": false,
                "required": ["attempts", "backoff", "base", "max"],
                "properties": {
                    "attempts": {"type": "integer", "minimum": 0},
                    "backoff": {"enum": ["exponential", "fixed"]},
                    "base": duration(),
                    "max": duration()
                }
            }
        }
    })
}

fn module_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "kind", "name"],
        "properties": {
            "id": {"type": "string", "minLength": 1},
            "kind": {"enum": ["preset", "profile", "accessor", "adapter", "source", "type"]},
            "name": {"type": "string", "minLength": 1},
            "description": {"type": "string"},
            "enabled": {"type": "boolean"},
            "labels": {"type": "object", "additionalProperties": {"type": "string"}},
            "extends": {"type": "array", "items": {"type": "string"}},
            "policy": policy_body(),
            "spec": {"type": "object"},
            "implementation": {"type": "string", "minLength": 1},
            "limits": limits(),
            "egress": {
                "type": "object",
                "additionalProperties": false,
                "required": ["profile"],
                "properties": {
                    "profile": {"type": "string", "minLength": 1},
                    "proxy": {"type": "string"}
                }
            },
            "paid": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "service": {"type": "string", "minLength": 1},
                    "budget": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["requests", "window"],
                        "properties": {
                            "requests": {"type": "integer", "minimum": 1},
                            "window": duration()
                        }
                    }
                }
            },
            "role": {"enum": ["search", "content"]},
            "consumes": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "kind": {"type": "string"},
                        "variant": {"type": "string"},
                        "part_role": {"type": "string"},
                        "media_type": {"type": "string"}
                    }
                }
            },
            "produces": {
                "type": "array",
                "items": {
                    "oneOf": [
                        {"const": "discovery"},
                        {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["kind"],
                            "properties": {
                                "kind": {"type": "string", "minLength": 1},
                                "variant": {"type": "string"}
                            }
                        }
                    ]
                }
            },
            "access": {
                "type": "object",
                "additionalProperties": false,
                "required": ["accessor", "locator"],
                "properties": {
                    "accessor": {"type": "string", "minLength": 1},
                    "locator": {
                        "type": "object",
                        "additionalProperties": false,
                        "minProperties": 1,
                        "maxProperties": 1,
                        "properties": {
                            "url": {"type": "string", "pattern": "^https?://.+$"},
                            "query": {"type": "string", "minLength": 1},
                            "endpoint": {"type": "string", "minLength": 1},
                            "channel": {"type": "string", "minLength": 1}
                        }
                    },
                    "render": {"enum": ["auto", "never", "always"]},
                    "spec": {"type": "object"}
                }
            },
            "pipeline": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "when": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "part_role": {"type": "string"},
                                "detected": {"type": "array", "items": {"type": "string"}},
                                "declared": {"type": "array", "items": {"type": "string"}},
                                "traits": {"type": "array", "items": {"type": "string"}}
                            }
                        },
                        "run": {"type": "array", "items": {"type": "string"}},
                        "run_if_accepts": {"type": "array", "items": {"type": "string"}},
                        "for": {"type": "string"},
                        "after": {"type": "string"},
                        "spec": {"type": "object"}
                    }
                }
            },
            "schedule": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "interval": duration(),
                    "floor": duration(),
                    "ceiling": duration(),
                    "adaptive": {"type": "boolean"},
                    "expectedPeriod": duration(),
                    "batchKey": {"type": "string"}
                }
            },
            "admission": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "allowUrlPrefixes": {"type": "array", "items": {"type": "string"}},
                    "denyUrlPrefixes": {"type": "array", "items": {"type": "string"}},
                    "publishedAfter": duration(),
                    "initialBackfill": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "limit": {"type": "integer", "minimum": 0},
                            "maxAge": duration()
                        }
                    },
                    "noise": {"type": "array", "items": {"type": "string"}},
                    "perRun": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["limit"],
                        "properties": {"limit": {"type": "integer", "minimum": 0}}
                    },
                    "daily": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["limit"],
                        "properties": {"limit": {"type": "integer", "minimum": 0}}
                    }
                }
            },
            "schema": {"type": "string", "minLength": 1}
        }
    })
}

fn system_schema() -> Value {
    let mut schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "kind", "policy"],
        "properties": {
            "id": {"const": "system"},
            "kind": {"const": "system"},
            "name": {"type": "string"},
            "description": {"type": "string"},
            "policy": policy_body(),
            "strata": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "gateway": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "backend": {"enum": ["sqlite", "postgres"]},
                            "path": {"type": "string"},
                            "url": {"type": "string"},
                            "extensions": {"type": "array", "items": {"type": "string"}},
                            "blob_backend": {
                                "enum": ["dir", "memory", "embedded", "rustfs", "s3"]
                            },
                            "blob_path": {"type": "string"},
                            "embedded": {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "bucket": {"type": "string", "minLength": 1},
                                    "prefix": {"type": "string"},
                                    "path": {"type": "string"}
                                }
                            },
                            "rustfs": {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "exe": {"type": "string"},
                                    "cache": {"type": "string"},
                                    "offline": {"type": "boolean"},
                                    "bucket": {"type": "string", "minLength": 1},
                                    "prefix": {"type": "string"},
                                    "path": {"type": "string"}
                                }
                            },
                            "s3": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["bucket"],
                                "properties": {
                                    "bucket": {"type": "string", "minLength": 1},
                                    "region": {"type": "string"},
                                    "endpoint": {"type": "string"},
                                    "prefix": {"type": "string"},
                                    "access_key_env": {"type": "string"},
                                    "secret_key_env": {"type": "string"}
                                }
                            }
                        }
                    },
                    "fts": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {"tokenizer": {"enum": ["unicode61", "trigram"]}}
                    },
                    "blob": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "inline_size_threshold": size(),
                            "dedicated_size_threshold": size()
                        }
                    },
                    "staging": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "dir": {"type": "string"},
                            "ttl": duration()
                        }
                    }
                }
            }
        }
    });
    schema["properties"]["strata"]["properties"]["gateway"]["properties"]["derivations"] =
        derivations_schema();
    schema
}

/// 派生声明。字段名与 `strata-gateway` 的 `DerivationSpec` 一致。
fn derivations_schema() -> Value {
    json!({
        "type": "array",
        "items": {
            "type": "object",
            "additionalProperties": false,
            "required": ["name", "from", "produces", "producer"],
            "properties": {
                "name": {"type": "string", "pattern": "^[!-~]+$"},
                "from": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["kind"],
                    "properties": {
                        "kind": {"enum": facet_kinds()},
                        "variant": {"type": "string", "pattern": "^[A-Za-z0-9][A-Za-z0-9._-]*$"},
                        "fields": {
                            "type": "array",
                            "items": {"enum": ["text", "struct", "blob"]}
                        }
                    }
                },
                "produces": {"enum": facet_kinds()},
                "variant": {"type": "string", "pattern": "^[A-Za-z0-9][A-Za-z0-9._-]*$"},
                "producer": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["module", "version", "configGeneration"],
                    "properties": {
                        "module": {"type": "string", "minLength": 1},
                        "version": {"type": "string", "minLength": 1},
                        "configGeneration": {"type": "integer", "minimum": 0}
                    }
                },
                "params": {"type": ["object", "null"]},
                "queue": {"enum": ["free", "paid", "render", "push"]},
                "accessor": {"type": "string", "minLength": 1},
                "priority": {"type": "integer"}
            }
        }
    })
}

/// 可声明的内置 facet 种类。`Other` 只能由代码构造。
fn facet_kinds() -> Value {
    json!([
        "raw",
        "html",
        "xml",
        "md",
        "text",
        "json",
        "yaml",
        "struct",
        "links",
        "image",
        "audio",
        "video",
        "file",
        "embedding",
        "regions"
    ])
}
