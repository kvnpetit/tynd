import { useEffect, useState } from "react";
import { createClient } from "bunview/client";
import type { AppCommands } from "../backend/main";
import "./App.css";

const client = createClient<AppCommands>();

interface SysInfo {
  platform: string;
  arch: string;
  hostname: string;
  cpus: number;
  totalMemoryGB: number;
  freeMemoryGB: number;
  bunVersion: string;
}

function App() {
  const [name, setName] = useState("World");
  const [greeting, setGreeting] = useState("");
  const [sum, setSum] = useState<number | null>(null);
  const [info, setInfo] = useState<SysInfo | null>(null);
  const [echo, setEcho] = useState("");

  useEffect(() => {
    client.rpc.getSystemInfo().then(setInfo).catch(console.error);
  }, []);

  return (
    <main style={{ fontFamily: "system-ui", padding: "2rem", maxWidth: 720, margin: "0 auto" }}>
      <h1>bunview + React + Vite</h1>

      <section style={{ marginTop: "2rem" }}>
        <h2>RPC — string argument</h2>
        <input value={name} onChange={(e) => setName(e.target.value)} />
        <button
          onClick={async () => setGreeting(await client.rpc.greet(name))}
          style={{ marginLeft: 8 }}
        >
          Call greet()
        </button>
        {greeting && <p><code>{greeting}</code></p>}
      </section>

      <section style={{ marginTop: "2rem" }}>
        <h2>RPC — object argument</h2>
        <button onClick={async () => setSum(await client.rpc.add({ a: 2, b: 40 }))}>
          Call add(&#123;a: 2, b: 40&#125;)
        </button>
        {sum !== null && <p>Result: <code>{sum}</code></p>}
      </section>

      <section style={{ marginTop: "2rem" }}>
        <h2>RPC — binary (Uint8Array roundtrip)</h2>
        <button onClick={async () => {
          const input = new TextEncoder().encode("Hello from frontend!");
          const out = await client.rpc.echoBytes(input);
          setEcho(new TextDecoder().decode(out));
        }}>
          Call echoBytes()
        </button>
        {echo && <p>Got back: <code>{echo}</code></p>}
      </section>

      {info && (
        <section style={{ marginTop: "2rem" }}>
          <h2>System info (via backend)</h2>
          <pre style={{ background: "#f5f5f5", padding: 12, borderRadius: 6 }}>
            {JSON.stringify(info, null, 2)}
          </pre>
        </section>
      )}
    </main>
  );
}

export default App;
