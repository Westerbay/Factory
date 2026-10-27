import { app } from "electron"
import { join } from "node:path"
import { writeFile } from "node:fs/promises"
import assert from "node:assert/strict"
import { createDesktop } from "../src/main/desktop.js"
const targetFile = process.argv[2],
  artifacts = process.argv[3],
  mode = process.argv[4] ?? "journey"
if (!targetFile || !artifacts)
  throw new Error("Use the guarded desktop smoke runner")
app.setPath("userData", join(artifacts, `profile-${mode}`))
const deadline = setTimeout(() => {
  console.error("Synthetic Electron smoke timed out")
  app.exit(1)
}, 45_000)
app
  .whenReady()
  .then(async () => {
    const desktop = await createDesktop({
      workerPath: join(import.meta.dirname, "worker.js"),
      workerArguments: [targetFile],
      chooseWorkspace: async () => artifacts,
      show: true,
    })
    const { window, client } = desktop
    async function evaluate(source: string) {
      try {
        return await window.webContents.executeJavaScript(source, true)
      } catch {
        // Expressions are synthetic test source only; never print provider output or IPC results.
        throw new Error(
          `Synthetic Electron expression failed: ${source.slice(0, 800)}`
        )
      }
    }
    async function wait(source: string) {
      for (let i = 0; i < 200; i++) {
        if (await evaluate(source)) return
        await new Promise((resolve) => setTimeout(resolve, 25))
      }
      throw new Error(`Synthetic condition failed: ${source}`)
    }
    await wait('document.querySelector("h1")?.textContent === "Factory"')
    await wait('document.body.innerText.includes("Provider ready")')
    assert.equal(await evaluate("typeof window.require"), "undefined")
    assert.equal(await evaluate("typeof window.process"), "undefined")
    const bridgeKeys = await evaluate(
      'Object.keys(window.factory).sort().join(",")'
    )
    assert.equal(bridgeKeys, "command,selectWorkspace,status,subscribe")
    const bad = await evaluate(
      'window.factory.command({type:"start-run",workspaceId:"forged",workspacePath:"/private",providerId:"synthetic-test-only",task:"bad"}).then(()=>false,()=>true)'
    )
    assert.equal(bad, true)
    if (mode === "development") {
      const csp = await evaluate(
        `document.querySelector('meta[http-equiv="Content-Security-Policy"]').content`
      )
      assert.match(csp, /script-src 'self' 'nonce-/)
      assert.doesNotMatch(csp, /unsafe-eval/)
      await desktop.shutdown()
      clearTimeout(deadline)
      console.log(
        "Synthetic Electron development renderer/CSP/secure bridge passed"
      )
      app.exit(0)
      return
    }
    if (mode === "recovery") {
      const recovered = await evaluate(
        'window.factory.command({type:"list-runs",offset:0,limit:50})'
      )
      assert.equal(recovered.type, "runs")
      assert.ok(
        recovered.runs.some(
          (run: { task: string; status: string }) =>
            run.task === "Synthetic hold for worker loss" &&
            run.status === "interrupted"
        )
      )
      assert.ok(
        recovered.runs.some(
          (run: { task: string; status: string }) =>
            run.task === "Synthetic desktop journey" &&
            run.status === "completed"
        )
      )
      await desktop.shutdown()
      clearTimeout(deadline)
      console.log("Synthetic Electron restart/history/recovery passed")
      app.exit(0)
      return
    }
    await evaluate(
      'Array.from(document.querySelectorAll("button")).find(button=>button.textContent === "Choose directory").click()'
    )
    await wait('document.body.innerText.includes("Synthetic test skill")')
    await evaluate(
      `(() => { const textarea=document.querySelector('textarea'); const setter=Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set; setter.call(textarea,'Synthetic desktop journey'); textarea.dispatchEvent(new Event('input',{bubbles:true})); })()`
    )
    await evaluate(
      'Array.from(document.querySelectorAll("button")).find(button=>button.textContent === "Start execution").click()'
    )
    await wait('document.querySelectorAll("form.request").length === 2')
    await wait('document.body.innerText.includes("Synthetic inert output")')
    assert.equal(
      await evaluate('document.querySelectorAll(".message img").length'),
      0
    )
    assert.equal(await evaluate("Boolean(window.syntheticInjection)"), false)
    assert.equal(
      await evaluate(
        'document.body.innerText.includes("Temporary synthetic output")'
      ),
      false
    )
    const first = 'document.querySelectorAll("form.request")[0]'
    const second = 'document.querySelectorAll("form.request")[1]'
    for (const form of [first, second]) {
      await evaluate(
        `(() => { const select=${form}.querySelector('select'); const setter=Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set; setter.call(select,'explicit'); select.dispatchEvent(new Event('change',{bubbles:true})); })()`
      )
      await evaluate(`${form}.querySelector('button[type="submit"]').click()`)
      if (form === first) {
        await wait('document.body.innerText.includes("Submitting —")')
        assert.equal(
          await evaluate(`${second}.querySelector('fieldset').disabled`),
          false
        )
      }
    }
    await evaluate(
      'Array.from(document.querySelectorAll("button")).find(button=>button.textContent === "Deny").click()'
    )
    await wait(
      'document.querySelector("#execution-title")?.closest("section").querySelector(":scope > p > strong")?.textContent === "completed"'
    )
    const persisted = await evaluate(
      'window.factory.command({type:"list-runs",offset:0,limit:50})'
    )
    const completed = persisted.runs.find(
      (run: { task: string }) => run.task === "Synthetic desktop journey"
    )
    assert.equal(completed.status, "completed")
    assert.equal(
      completed.questions.every(
        (question: { status: string }) => question.status === "resolved"
      ),
      true
    )
    await evaluate(
      'document.querySelector("#execution-title").scrollIntoView({block:"start"})'
    )
    await evaluate(
      "new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))"
    )
    await writeFile(
      join(artifacts, "desktop-execution.png"),
      (await window.webContents.capturePage()).toPNG()
    )
    await evaluate("window.scrollTo(0, 0)")
    window.webContents.focus()
    await evaluate('document.querySelector("button").focus()')
    window.webContents.sendInputEvent({ type: "keyDown", keyCode: "Tab" })
    window.webContents.sendInputEvent({ type: "keyUp", keyCode: "Tab" })
    assert.equal(
      await evaluate(
        'document.activeElement !== document.body && document.activeElement !== document.querySelector("button")'
      ),
      true
    )
    await evaluate(
      "new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))"
    )
    await writeFile(
      join(artifacts, "desktop-wide.png"),
      (await window.webContents.capturePage()).toPNG()
    )
    window.setSize(520, 900)
    await evaluate(
      "new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))"
    )
    assert.equal(
      await evaluate(
        "document.documentElement.scrollWidth <= window.innerWidth"
      ),
      true
    )
    await writeFile(
      join(artifacts, "desktop-narrow.png"),
      (await window.webContents.capturePage()).toPNG()
    )
    window.setSize(1180, 850)
    const workspace = await evaluate("window.factory.selectWorkspace()")
    const held = await evaluate(
      `window.factory.command({type:"start-run",workspaceId:${JSON.stringify(workspace.workspace.id)},providerId:"synthetic-test-only",task:"Synthetic hold for retry stop"})`
    )
    assert.equal(held.type, "run")
    await wait(
      `Boolean(document.querySelector('button[data-run-id="${held.run.id}"]'))`
    )
    await evaluate(
      `document.querySelector('button[data-run-id="${held.run.id}"]').click()`
    )
    await wait(
      'document.querySelector("#execution-title")?.closest("section").querySelector(":scope > p > strong")?.textContent === "running"'
    )
    const stopButton =
      'Array.from(document.querySelectorAll("button")).find(button=>button.textContent === "Stop execution")'
    await evaluate(`${stopButton}.click()`)
    await wait(
      `document.querySelector("#execution-title")?.closest("section").querySelector(":scope > p > strong")?.textContent === "cancelling" && ${stopButton}.disabled === false`
    )
    await evaluate(`${stopButton}.click()`)
    await wait(
      'document.querySelector("#execution-title")?.closest("section").querySelector(":scope > p > strong")?.textContent === "interrupted"'
    )
    const continued = await evaluate(
      `window.factory.command({type:"continue-run",runId:${JSON.stringify(completed.id)},agentId:${JSON.stringify(completed.rootAgentId)},text:"Synthetic hold for worker loss"})`
    )
    assert.equal(continued.type, "run")
    assert.notEqual(continued.run.id, completed.id)
    assert.equal(continued.run.continuationOf.runId, completed.id)
    // Deliberately terminate only this test utility; synthetic provider has no native subprocess.
    desktop.process.kill()
    await wait('document.body.innerText.includes("cleanup is unconfirmed")')
    assert.equal(client.status().state, "unavailable")
    clearTimeout(deadline)
    console.log(
      "Synthetic Electron bridge/UI/multi-question/permission/output/stop/continuation/worker-loss passed"
    )
    app.exit(0)
  })
  .catch((error: unknown) => {
    clearTimeout(deadline)
    console.error(error)
    app.exit(1)
  })
