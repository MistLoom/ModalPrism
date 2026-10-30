// ability.yml 声明子集解析测试。
package leaf_ability_sdk

import std.collection.ArrayList

import std.unittest.*
import std.unittest.testmacro.*

internal const SAMPLE_YAML: String = """
framework_version: "cangjie@1"
id: algo.echo
open: true
protocols: [tools, cli]
depends:
  - id: dep.store
    alias: store
stateful: false
methods:
  - id: run
    description: 回显输入文本
  - id: ping
events:
  - name: pong
    dir: emit
"""

@Test
public func testLoadAbilityYaml(): Unit {
    let decl = loadAbilityYaml(SAMPLE_YAML)
    @Assert(decl.id == "algo.echo")
    @Assert(decl.frameworkVersion == "cangjie@1")
    @Assert(decl.stateful == false)
    @Assert(decl.methods.size == 2)
    @Assert(decl.methods[0][0] == "run")
    @Assert(decl.methods[0][1] == "回显输入文本")
    @Assert(decl.methods[1][0] == "ping")
    @Assert(decl.events.size == 1)
    @Assert(decl.events[0][0] == "pong")
    @Assert(decl.events[0][1] == "emit")
    @Assert(decl.depends.size == 1)
    @Assert(decl.depends[0][0] == "dep.store")
    @Assert(decl.depends[0][1] == Some("store"))
}

@Test
public func testLoadAbilityYamlMissingIdThrows(): Unit {
    var threw = false
    try {
        loadAbilityYaml("framework_version: \"1\"\n")
    } catch (e: SdkError) {
        threw = true
    }
    @Assert(threw)
}

@Test
public func testStatefulAndEmptyDepends(): Unit {
    let text = "id: a.b\nstateful: true\ndepends: []\nmethods:\n  - id: go\n"
    let decl = loadAbilityYaml(text)
    @Assert(decl.stateful)
    @Assert(decl.depends.size == 0)
    @Assert(decl.methods.size == 1)
}

@Test
public func testSpecManifest(): Unit {
    let spec = AbilitySpec.must("algo.echo")
    spec.dependsAlias("dep.store", "store")
    spec.eventEmit("pong", "回", jObjOf([("type", JStr("object"))]))
    spec.on("run", "回显", jObjOf([("type", JStr("object"))]), { _ctx, args => args })
    let m = spec.manifest()
    @Assert(jAsStr(jObjGet(m, "id")) == Some("algo.echo"))
    @Assert(jAsBool(jObjGet(m, "stateful")) == Some(false))
    let methods = match (jAsArr(jObjGet(m, "methods"))) {
        case Some(l) => l
        case None => @Assert(false); ArrayList<JsonValue>()
    }
    @Assert(methods.size == 1)
    @Assert(jAsStr(jObjGet(methods[0], "id")) == Some("run"))
    @Assert(jIsNull(jObjGet(methods[0], "output_schema")))
    let depends = match (jAsArr(jObjGet(m, "depends"))) {
        case Some(l) => l
        case None => @Assert(false); ArrayList<JsonValue>()
    }
    @Assert(jAsStr(jObjGet(depends[0], "alias")) == Some("store"))
    @Assert(jAsBool(jObjGet(depends[0], "optional")) == Some(false))
    let events = match (jAsArr(jObjGet(m, "events"))) {
        case Some(l) => l
        case None => @Assert(false); ArrayList<JsonValue>()
    }
    @Assert(jAsStr(jObjGet(events[0], "dir")) == Some("emit"))
}
