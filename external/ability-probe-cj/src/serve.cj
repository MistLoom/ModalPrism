// probe 外部仓颉 Guest：镜像 ability-probe-ts / ability-probe-py（ping/pong emit + pending 长任务）。
package probe_cj

import std.collection.HashMap
import std.sync.*

import leaf_ability_sdk.*

main(): Int64 {
    let handlers = HashMap<String, Handler>()
    handlers.add("ping", { ctx, _args =>
        ctx.emit("pong", jObjOf([("from", JStr("probe"))]))
        return jObjOf([("ok", JBool(true)), ("from", JStr("external-cj"))])
    })
    handlers.add("slow", { _ctx, args =>
        let ms = match (jAsNum(jObjGet(args, "ms"))) {
            case Some(v) => Int64(v)
            case None => 150
        }
        let fail = match (jAsBool(jObjGet(args, "fail"))) {
            case Some(b) => b
            case None => false
        }
        return pending({ =>
            sleep(Duration.millisecond * Int64(ms))
            if (fail) {
                throw SdkError("boom", "failed")
            }
            return jObjOf([("slept", JNum(Float64(ms)))])
        })
    })
    serveAbility(handlers)
    return 0
}
