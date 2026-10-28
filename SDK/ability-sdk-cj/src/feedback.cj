// Guest 向宿主提交反馈（对齐 CtxOp::Feedback 与 lib/feedback.js）。
package leaf_ability_sdk

import std.collection.ArrayList
import std.time.*

var feedbackIdSeq: Int64 = 0

func nextFeedbackId(): String {
    feedbackIdSeq += 1
    return "fb${nowMillis()}-${feedbackIdSeq}"
}

// 本版 std.time 无纪元换算 API，用 DateTime 字段自行换算纪元毫秒。
func nowMillis(): Int64 {
    let t = DateTime.now()
    let monthNum: Int64 = match (t.month) {
        case January => 1
        case February => 2
        case March => 3
        case April => 4
        case May => 5
        case June => 6
        case July => 7
        case August => 8
        case September => 9
        case October => 10
        case November => 11
        case December => 12
    }
    let days = daysFromCivil(t.year, monthNum, Int64(t.dayOfMonth))
    let secs = days * 86400 + Int64(t.hour) * 3600 + Int64(t.minute) * 60 + Int64(t.second)
    return secs * 1000
}

// civil-from-days（Howard Hinnant 算法）的逆运算。
func daysFromCivil(y0: Int64, m: Int64, d: Int64): Int64 {
    var y = y0
    if (m <= 2) {
        y -= 1
    }
    let era = if (y >= 0) { y } else { y - 399 } / 400
    let yoe = y - era * 400
    let mp = (m + 9) % 12
    let doy = (153 * mp + 2) / 5 + d - 1
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
    return era * 146097 + doe - 719468
}

public func submitFeedback(ctx: GuestCtx, tags: ArrayList<String>, desc: String): Unit {
    let tagList = ArrayList<JsonValue>()
    for (t in tags) {
        tagList.add(JStr(t))
    }
    let record = jObjOf([
        ("id", JStr(nextFeedbackId())),
        ("at", JNum(Float64(nowMillis()))),
        ("loc", jObjOf([
            ("ability", JStr(ctx.instance ?? "guest")),
            ("module", JStr("serve")),
        ])),
        ("tags", JArr(tagList)),
        ("desc", JStr(desc)),
        ("chain_id", JStr(ctx.chainId ?? "detached")),
        ("span", JNum(Float64(ctx.span))),
    ])
    ctx.ctxOp(jObjOf([
        ("op", JStr("feedback")),
        ("record", record),
    ]))
    return
}
