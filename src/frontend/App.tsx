import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";

function App() {
  const [name, setName] = useState("");
  const [greeting, setGreeting] = useState("");
  const [isTauri, setIsTauri] = useState<boolean | null>(null);

  async function handleGreet() {
    try {
      const result = await invoke<string>("greet", { name });
      setGreeting(result);
      setIsTauri(true);
    } catch {
      setGreeting("Running in browser mode (no Tauri backend).");
      setIsTauri(false);
    }
  }

  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-6 bg-background p-8 text-foreground">
      <div className="flex flex-col items-center gap-2">
        <h1 className="text-4xl font-bold tracking-tight">Dash Evo Tool</h1>
        <p className="text-muted-foreground">Tauri 2.0 + React Frontend</p>
        {isTauri !== null && (
          <Badge variant={isTauri ? "default" : "secondary"}>
            {isTauri ? "Tauri Backend Connected" : "Browser Only"}
          </Badge>
        )}
      </div>

      <Card className="w-full max-w-md">
        <CardHeader>
          <CardTitle>IPC Test</CardTitle>
          <CardDescription>
            Test the Tauri IPC bridge by sending a greet command to the Rust
            backend.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <div className="flex gap-2">
            <Input
              placeholder="Enter your name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleGreet();
              }}
            />
            <Button onClick={handleGreet}>Greet</Button>
          </div>
          {greeting && (
            <p className="rounded-md bg-muted p-3 text-sm">{greeting}</p>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

export default App;
