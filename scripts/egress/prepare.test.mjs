import test from "node:test"
import assert from "node:assert/strict"
import { hostAddresses, prepareEgress } from "./prepare.mjs"
import { mkdtemp, readFile, writeFile, unlink, rmdir } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { randomUUID } from "node:crypto"

test("host deny addresses include every IPv4 interface and cannot contain rule text", () => {
  assert.deepEqual(
    hostAddresses({
      eth0: [
        { family: "IPv4", address: "8.8.8.8" },
        { family: "IPv4", address: "8.8.8.8" },
        { family: "IPv4", address: "1.2.3.4; accept" },
        { family: "IPv6", address: "::1" },
      ],
    }),
    ["127.0.0.1", "8.8.8.8"]
  )
})

test("preparation preserves the previous contract until incompatible proxy maintenance is complete", async () => {
  const directory = await mkdtemp(join(tmpdir(), "factory-egress-prepare-"))
  const installationId = randomUUID()
  const containerId = "d".repeat(64)
  let imageId = `sha256:${"a".repeat(64)}`
  let proxy
  const runDocker = (args) => {
    if (args[0] === "build") return ""
    if (args[0] === "version")
      return JSON.stringify({ Os: "linux", ApiVersion: "1.56" })
    if (args[0] === "image")
      return JSON.stringify([
        {
          Id: imageId,
          Os: "linux",
          Config: {
            User: "0:0",
            Labels: {
              "io.factory.egress.image": "1",
              "io.factory.egress.squid": "7.7",
            },
          },
        },
      ])
    if (args[0] === "container" && args[1] === "ls")
      return proxy ? containerId : ""
    if (args[0] === "container" && args[1] === "inspect")
      return JSON.stringify([proxy])
    throw new Error(`Unexpected fixture operation: ${args[0]}`)
  }
  try {
    await writeFile(
      join(directory, "installation.json"),
      JSON.stringify({ installationId })
    )
    const first = await prepareEgress(directory, runDocker)
    proxy = {
      Id: containerId,
      Name: `/factory-egress-${installationId}`,
      Image: imageId,
      Config: {
        Image: imageId,
        Env: [
          `FACTORY_HOST_IPV4=${first.hostIpv4.join(",")}`,
          "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        ],
        Labels: {
          "io.factory.installation": installationId,
          "io.factory.egress.policy": "1",
          "io.factory.egress.role": "shared",
          "io.factory.egress.image": "1",
          "io.factory.egress.squid": "7.7",
        },
      },
    }
    assert.deepEqual(await prepareEgress(directory, runDocker), first)
    const initialBytes = await readFile(join(directory, "egress.json"), "utf8")
    imageId = `sha256:${"b".repeat(64)}`
    await assert.rejects(
      prepareEgress(directory, runDocker),
      /existing metadata was preserved/
    )
    assert.equal(
      await readFile(join(directory, "egress.json"), "utf8"),
      initialBytes
    )
    imageId = first.imageId
    const oldAddresses = JSON.stringify({
      ...first,
      hostIpv4: ["127.0.0.1", "8.8.8.8"],
    })
    await writeFile(join(directory, "egress.json"), oldAddresses)
    await assert.rejects(
      prepareEgress(directory, runDocker),
      /verified proxy maintenance/
    )
    assert.equal(
      await readFile(join(directory, "egress.json"), "utf8"),
      oldAddresses
    )
    await writeFile(join(directory, "egress.json"), initialBytes)
    proxy.Config.Labels["io.factory.installation"] = randomUUID()
    await assert.rejects(
      prepareEgress(directory, runDocker),
      /existing metadata was preserved/
    )
    assert.equal(
      await readFile(join(directory, "egress.json"), "utf8"),
      initialBytes
    )
    await unlink(join(directory, "egress.json"))
    await assert.rejects(
      prepareEgress(directory, runDocker),
      /existing metadata was preserved/
    )
    proxy = undefined // Simulates the operator's explicitly verified removal, never done by preparation.
    const resumed = await prepareEgress(directory, runDocker)
    assert.deepEqual(resumed, first)
  } finally {
    await unlink(join(directory, "egress.json"))
    await unlink(join(directory, "installation.json"))
    await rmdir(directory)
  }
})
