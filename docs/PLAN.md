# Plan: extensión de VS Code para VRL (Vector Remap Language)

Objetivo: una extensión que empiece por resaltado y acabe con diagnósticos reales del
compilador de VRL, hover/autocompletado generados desde la stdlib, y ejecución contra
eventos de muestra. Cada fase es utilizable por sí sola y no bloquea la siguiente.

Nombre de trabajo: `vrl-tools`. Publisher: el tuyo. Licencia sugerida: MIT (el crate
`vrl` es MPL-2.0, así que si acabas enlazando el crate revisa la sección de licencias
al final).

---

## Decisiones de arquitectura (tomar antes de la Fase 3)

### D1 — ¿WASM o binario nativo?

El crate `vrl` compila a `wasm32-unknown-unknown` sin las funciones que tocan I/O
(`parse_grok`, `parse_groks`, `log`, `get_hostname`, `reverse_dns`, `dns_lookup`,
`http_request`, `validate_json_schema`). Para **comprobación estática** eso da igual:
esas funciones compilan bien, solo abortan en runtime. Solo te afecta si en la Fase 5
quieres ejecutar programas de verdad dentro del editor.

- **Ruta A — WASM + TypeScript (recomendada para empezar).** Compilas un módulo wasm
  minúsculo que expone `vrl_check(source, sample_event?) -> JSON` y consumes eso desde
  TypeScript con la API `vscode.languages.createDiagnosticCollection`. No hay servidor
  LSP, no hay proceso hijo, no hay matriz de compilación cruzada: un solo `.vsix` que
  funciona en Windows, macOS, Linux y en `vscode.dev`. Es bastante menos código.
- **Ruta B — servidor LSP nativo en Rust (`tower-lsp`).** Solo merece la pena si
  quieres el mismo motor en Neovim / Helix / IntelliJ. Coste: cross-compilar para 5–6
  targets, firmar el binario en macOS, y gestionar el ciclo de vida del proceso.

**Recomendación:** Ruta A, pero factorizando el checker en su propio crate
(`vrl-check-core`) que no sepa nada de wasm. Así la Ruta B en el futuro es un wrapper
`tower-lsp` de ~200 líneas sobre el mismo core, no una reescritura.

### D2 — Versión de VRL

La stdlib cambia entre versiones (funciones nuevas, parámetros, deprecaciones). Fija
`vrl = "=X.Y.Z"` en el `Cargo.toml` y **muestra esa versión en la barra de estado**,
porque si tu Vector en producción es más antiguo verás falsos positivos/negativos.
Comprueba qué versión del crate usa tu Vector antes de fijarla.

Idea para más adelante: `vrl-tools.vrlVersion` como setting, con varios wasm empaquetados.

### D3 — El tipo de `.`

Por defecto el compilador asume que el evento externo es de tipo desconocido/cualquiera,
lo que genera muchos `unhandled fallible assignment` legítimos pero también hace que la
inferencia sea pobre. En la Fase 5 esto se resuelve alimentando un evento de muestra
(ver más abajo). Decide desde el principio que el core acepte un `sample_event`
opcional, aunque al inicio siempre le pases `None`.

---

## Fase 0 — Scaffolding (30 min)

```
npm install -g yo generator-code
yo code            # → New Extension (TypeScript), nombre vrl-tools
```

Estructura objetivo:

```
vrl-tools/
  package.json                 # manifest de la extensión
  language-configuration.json
  syntaxes/
    vrl.tmLanguage.json
    vrl-injection-yaml.json    # Fase 2
  snippets/vrl.json            # Fase 2
  src/extension.ts
  crates/
    vrl-check-core/            # Fase 3
    vrl-check-wasm/            # Fase 3
  media/icon.png
```

En `package.json`, contribución de lenguaje:

```json
"contributes": {
  "languages": [{
    "id": "vrl",
    "aliases": ["VRL", "Vector Remap Language"],
    "extensions": [".vrl"],
    "configuration": "./language-configuration.json"
  }],
  "grammars": [{
    "language": "vrl",
    "scopeName": "source.vrl",
    "path": "./syntaxes/vrl.tmLanguage.json"
  }]
}
```

**Criterio de salida:** `F5` abre una Extension Development Host y un `.vrl` se detecta
como lenguaje VRL (aunque aún salga en blanco y negro).

---

## Fase 1 — Gramática TextMate + configuración de lenguaje (medio día)

Aquí es donde fallan las extensiones existentes, así que conviene ser meticuloso. Lista
de construcciones que **tienen** que estar cubiertas:

| Construcción | Ejemplo | Scope sugerido |
|---|---|---|
| Path de evento | `.foo.bar`, `.["a-b"]`, `.list[0]` | `variable.other.property.vrl` |
| Path raíz | `.` a solas | `variable.language.vrl` |
| Path de metadata | `%vector.ingest_timestamp` | `variable.other.metadata.vrl` |
| Coalescencia de path | `.foo.(a \| b)` | operador dentro del path |
| Llamada infalible | `parse_json!(...)` — el `!` es parte del nombre | `keyword.operator.fallible.vrl` |
| Asignación con error | `x, err = parse_json(.message)` | destructuring |
| Operador `??` | `.a ?? .b ?? "default"` | `keyword.operator.coalesce.vrl` |
| String | `"texto \n"` con escapes | `string.quoted.double.vrl` |
| String literal | `s'sin escapes'` | `string.quoted.single.raw.vrl` |
| Regex | `r'^\d+$'` | `string.regexp.vrl` |
| Timestamp | `t'2021-01-01T00:00:00Z'` | `constant.other.timestamp.vrl` |
| Interpolación | `"{{ .field }}"` en contexto de plantilla | `meta.embedded` |
| Comentario | `# ...` | `comment.line.number-sign.vrl` |
| Palabras clave | `if`, `else`, `abort`, `return`, `null`, `true`, `false` | `keyword.control.vrl` |
| Closures | `for_each(.obj) -> \|k, v\| { ... }` | parámetros como `variable.parameter` |
| Números | `42`, `3.14`, negativos | `constant.numeric.vrl` |
| Funciones stdlib | `parse_syslog`, `to_int`, `del`, `exists`… | `support.function.vrl` |

**Truco:** no escribas a mano la lista de funciones stdlib. Genera esa regex desde
`docs/generated` del repo `vectordotdev/vrl` con un script en `scripts/gen-grammar.ts`
que se ejecute en `npm run build`. Así la gramática se actualiza sola al subir de versión.

**Errores típicos a evitar:**
- Que el `.` de un path se coma el `.` de un número decimal.
- Que `!` de negación (`!exists(.x)`) se confunda con el `!` de infalibilidad.
- Que `r'...'` se resalte como string normal y los escapes de regex rompan el resaltado.
- No cubrir paths entre comillas: `."@timestamp"` es muy común en logs de ECS.

`language-configuration.json` mínimo: comentarios `#`, pares `{}` `[]` `()` `""` `''`,
`autoClosingPairs` excluyendo comillas dentro de comentarios y strings, e `indentationRules`
para que las llaves indenten.

**Criterio de salida:** abres tus parsers de producción más feos y todo se resalta
correctamente de principio a fin, sin zonas que se "apaguen".

---

## Fase 2 — Snippets e inyección en YAML/TOML (medio día)

**Snippets** (`snippets/vrl.json`) — arranca con los patrones que repites tú:
parseo JSON con manejo de error, `parse_syslog`, `parse_key_value`, `parse_grok`,
normalización de timestamp, bloque de coerción de tipos, `for_each` sobre objeto,
plantilla de mapeo a ECS/OCSF. Estos sí a mano: el valor está en que sean *tus*
patrones, no los del manual.

**Inyección en configs de Vector** — esto es lo que más se nota en el día a día. Una
gramática de inyección que resalte como VRL el contenido de `source: |` dentro de un
transform `type: remap`, y lo mismo en TOML (`source = """ ... """`).

```json
{
  "scopeName": "vrl.injection.yaml",
  "injectionSelector": "L:source.yaml -string -comment",
  "patterns": [{ "include": "#vrl-source-block" }]
}
```

Aviso honesto: TextMate no tiene contexto semántico, así que no puedes verificar de
verdad que el bloque esté bajo `type: remap`. Lo pragmático es disparar sobre la clave
`source:` seguida de `|` o `|-` en cualquier YAML que ya se haya detectado como config
de Vector, y dejarlo tras un setting `vrl-tools.injectIntoYaml` por si molesta.

Considera además `"filenames"` / `"filenamePatterns"` para autodetectar
`vector.yaml`, `vector.toml`, `**/vector/*.yaml`.

**Criterio de salida:** publicable como v0.1.0. Ya es mejor que lo que hay en el
marketplace.

---

## Fase 3 — Diagnósticos reales del compilador (1–2 días) ⭐

El núcleo del proyecto. La clave: **no escribas un parser**. Usa el compilador real.

### 3.1 — `crates/vrl-check-core`

```toml
[dependencies]
vrl = { version = "=X.Y.Z", default-features = false,
        features = ["compiler", "parser", "stdlib", "diagnostic", "value", "path", "core"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

La forma de uso confirmada de la API (verifica firmas exactas en docs.rs para la
versión que fijes, el crate aún hace cambios de superficie):

```rust
let fns = vrl::stdlib::all();
match vrl::compiler::compile(&source, &fns) {
    Ok(result) => {
        // result.program, result.warnings (DiagnosticList)
    }
    Err(diagnostics) => {
        // DiagnosticList → tu tipo serializable
    }
}
```

Cada `Diagnostic` trae `severity`, `code`, `message`, `notes` y `labels`; cada `Label`
trae `message`, si es `primary`, y un `Span` con offsets **de byte**.

### 3.2 — Offsets de byte → posiciones LSP

Este es el punto donde más gente se equivoca y produce subrayados descolocados.
VRL da bytes; VS Code quiere `(línea, carácter)` en **UTF-16**. Con logs reales llenos
de acentos, emojis y CJK esto rompe seguro.

Construye una tabla de línea/columna una vez por documento y conviértela bien.
Escribe tests con un fichero que contenga `ñ`, `→` y un emoji fuera del BMP antes de
dar la fase por buena.

### 3.3 — `crates/vrl-check-wasm`

Un wrapper `wasm-bindgen` que expone:

```rust
#[wasm_bindgen]
pub fn check(source: &str, sample_event_json: Option<String>) -> String // JSON
```

Build: `wasm-pack build --target nodejs`. Empaqueta el `.wasm` en el `.vsix`.

### 3.4 — Lado TypeScript

- `vscode.languages.createDiagnosticCollection("vrl")`
- Recompilar en `onDidChangeTextDocument` con debounce de ~300 ms
- Mapear severidades: errores → `Error`, warnings del compilador → `Warning`
- Meter `notes` del diagnóstico en `relatedInformation`, no concatenados en el mensaje
- Usar `diagnostic.code` con el `Urls` del crate para que el link "ver documentación"
  del hover lleve a la página del error

**Criterio de salida:** escribes `parse_json(.message)` sin `!` ni manejo de error y ves
el subrayado con el mensaje exacto que te daría `vector validate`, en el sitio correcto,
antes de guardar el fichero. Aquí es donde la extensión deja de ser cosmética.

---

## Fase 4 — Hover, autocompletado y signature help (1 día)

Otra vez: **generado, no escrito a mano**. El trait `Function` de VRL expone
`identifier()`, `parameters()` y `examples()`, y el repo tiene `docs/generated` con las
descripciones. Un binario `xtask` que recorra `vrl::stdlib::all()` y vuelque un
`functions.json` con nombre, parámetros (nombre, tipos aceptados, requerido), si es
fallible, tipo de retorno y ejemplos.

Con ese JSON salen tres features casi gratis:

- **Hover** sobre una función: firma, descripción, si es fallible, ejemplo.
- **Completion** (`CompletionItemProvider`): al escribir, lista de funciones filtrada;
  `insertText` como snippet con los parámetros como tabstops; y sufijo `!` automático
  para las fallibles cuando estás en posición de asignación.
- **Signature help** (`SignatureHelpProvider`): al abrir el paréntesis, parámetro actual
  resaltado. Es lo que más tiempo ahorra con funciones tipo `parse_timestamp`.

Extra barato: completion de paths a partir de los paths ya usados en el fichero. Suena
tonto, pero acertar `.` + Ctrl+Espacio con los campos que ya has tocado evita muchos
typos silenciosos.

---

## Fase 5 — Ejecución y tipos reales (1–2 días, opcional)

Lo que convierte la extensión en un entorno de trabajo:

1. **Comando `VRL: Run on sample event`** — abres un `.vrl`, eliges un JSON de muestra
   (`.vrl.sample.json` junto al fichero, o el porta­papeles), y ves el evento de salida
   en un panel al lado. Con el runtime del crate: `vrl::compiler::runtime::Runtime` y un
   `TargetValue`. Es el playground de vector.dev pero local y sobre tus datos.
2. **Inferencia con el evento de muestra** — al pasar el shape del sample al estado de
   tipos del compilador, los diagnósticos pasan de "podría fallar" a saber de verdad qué
   campos existen y de qué tipo son. Esto es lo que ni el playground te da cómodamente.
3. **CodeLens** por bloque con el tiempo de ejecución, si te da por optimizar parsers.
4. **Semantic tokens** — colorear distinto una variable local frente a un path frente a
   una función, ya con información del AST en vez de regex.

Alternativa sin Rust para el punto 1: llamar al binario `vector vrl` que ya tienes
instalado y parsear su salida. Más frágil, pero se monta en una tarde si quieres el
resultado antes que la elegancia.

---

## Empaquetado y distribución

- `vsce package` → `.vsix` instalable en local (`code --install-extension`). Con esto ya
  lo usas tú y en el equipo sin publicar nada.
- Si publicas: cuenta de Azure DevOps, publisher en el marketplace, y `vsce publish`.
- CI en GitHub Actions: build del wasm + `vsce package` + adjuntar el `.vsix` a la
  release del tag. Así no dependes de tu máquina.
- Añade icono, README con GIF del resaltado y de los diagnósticos, y
  `categories: ["Programming Languages", "Linters"]`.

**Licencias:** el crate `vrl` es MPL-2.0. Enlazarlo desde tu código no obliga a abrir tu
código, pero si modificas ficheros del propio crate esos ficheros siguen bajo MPL.
Mantén tu core como consumidor del crate sin tocarlo y no tendrás problema. Incluye el
aviso de licencia de terceros en el `.vsix`.

---

## Riesgos y cosas que se van a torcer

| Riesgo | Mitigación |
|---|---|
| La API del crate `vrl` cambia entre versiones | Fija versión exacta; el core aislado hace que el arreglo sea de un fichero |
| Offsets byte↔UTF-16 mal convertidos | Tests con acentos/CJK/emoji desde el primer día de la Fase 3 |
| Tamaño del `.wasm` (la stdlib entera no es pequeña) | `opt-level="z"`, `lto`, `wasm-opt -Oz`; quitar features `datadog_*` si no los usas |
| Latencia al recompilar en cada tecla | Debounce + cancelar la compilación anterior; VRL compila rapidísimo, no debería ser problema |
| Desfase entre la versión de VRL de la extensión y la de tu Vector | Mostrar la versión en la barra de estado; documentarlo en el README |
| Falsos positivos por no conocer el shape de `.` | Fase 5.2; mientras tanto, dejar rebajar esos diagnósticos a warning por setting |

---

## Orden de ataque sugerido

1. Fase 0 + 1 en una sesión → ya se ve bonito, motiva.
2. Fase 2 → v0.1.0, instálalo y úsalo una semana. Las fricciones reales que encuentres
   deben reordenar el resto del plan.
3. Fase 3 → v0.2.0, el salto de valor.
4. Fase 4 → v0.3.0.
5. Fase 5 solo si después de usarlo un mes lo sigues echando de menos.

## Checklist para arrancar en el PC

- [ ] Node LTS + `npm i -g yo generator-code @vscode/vsce`
- [ ] Rust estable + `wasm-pack` + target `wasm32-unknown-unknown`
- [ ] Comprobar la versión del crate `vrl` que usa tu Vector actual
- [ ] Repo git con la estructura de la Fase 0
- [ ] Reunir 5–10 ficheros `.vrl` reales tuyos como corpus de pruebas (los más feos)
- [ ] Un JSON de muestra por cada uno, para la Fase 5
