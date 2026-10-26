import { messages } from "./messages"

export function App() {
  return (
    <main>
      <header>
        <p>{messages.subtitle}</p>
        <h1>{messages.title}</h1>
      </header>
      <section aria-labelledby="foundation-title">
        <h2 id="foundation-title">{messages.heading}</h2>
        <p>{messages.description}</p>
        <p className="status">{messages.status}</p>
      </section>
      <footer>{messages.architecture}</footer>
    </main>
  )
}
