import "./theme";
import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import App from "./App";
import { isTauri } from "./ipc/api";
import "./styles.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
      staleTime: 10_000,
      retry: 1,
    },
  },
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      {isTauri ? <App /> : (
        <main className="empty-state">
          <h1>Open the desktop app to edit your timeline.</h1>
        </main>
      )}
    </QueryClientProvider>
  </React.StrictMode>,
);
