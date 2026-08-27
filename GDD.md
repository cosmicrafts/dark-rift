# Game Design Document (GDD): Dark Rift // Singularity Vacuum

---

## 1. Concepto Central
Un juego incremental / idle físico donde controlas el nacimiento y expansión de una singularidad cósmica (agujero negro) que devora objetos celestes tangibles en el espacio profundo.

---

## 2. Objetos Celestes Reales (No Partículas Decorativas)
Cada objeto en pantalla es una **entidad física 3D individual**, con silueta clara, masa definida, collider de clic y física de atracción hacia el vórtice:

| Objeto Celeste | Apariencia Visual | Masa Base ($M$) | Comportamiento / Características |
| :--- | :--- | :--- | :--- |
| **Fragmento Cuántico (Pebble)** | Roca pequeña brillante con gas cian | $1.0\text{ kg}$ | Abundante, fácil de absorber, spawnea desde los bordes. |
| **Meteorito de Condrita** | Roca incandescente con estela de fuego | $5.0\text{ kg}$ | Mayor velocidad orbital, requiere ser interceptado. |
| **Asteroide Denso (Hierro-Níquel)** | Roca metálica pesada oscura | $25.0\text{ kg}$ | Requiere mayor fuerza de succión para ser arrastrado al centro. |
| **Cristal de Antimateria** | Fragmento cristalino violeta brillante | $100.0\text{ kg}$ | Raro ($5\%$). Al ser devorado detona un pulso que atrae instantáneamente todo en pantalla. |
| **Proto-Luna / Planetoide** | Macro-cuerpo esférico con cráteres | $1,000.0\text{ kg}$ | Muy pesado. Requiere radio de succión y potencia mejorados. |

---

## 3. El Vórtice Gravitacional (Agujero Negro)
* Ubicado en el campo espacial interactivo con efecto de lente gravitacional y disco de acreción en Three.js.
* **Mecánica Activa del Jugador:**
  1. **Clic directo sobre un objeto:** Lo engancha con un hilo gravitacional y lo lanza a toda velocidad al centro.
  2. **Clic / Pulso en el Vórtice:** Emite una onda de choque gravitacional que duplica temporalmente la velocidad de succión de todos los objetos en pantalla.

---

## 4. Árbol de Mejoras del Panel Principal (Inspirado en Vacuum Idle)

| # | Nombre de Mejora | Icono / Rol | Efecto en Juego |
| :---: | :--- | :---: | :--- |
| **1** | **Suction Power (Fuerza de Gravedad)** | 🌀 | Incrementa la velocidad con la que los objetos caen al vórtice. |
| **2** | **Event Horizon (Radio de Succión)** | 🕳️ | Agranda el radio de captura automática alrededor del agujero negro. |
| **3** | **Matter Density (Valor de Masa)** | 💰 | Multiplica la masa obtenida por cada objeto devorado ($\times 1.25, \times 1.5, \dots$). |
| **4** | **Inflow Rate (Frecuencia de Spawn)** | ⏱️ | Genera nuevos objetos celestes con mayor rapidez. |
| **5** | **Exotic Matter Chance (Probabilidad Rara)** | ⭐ | Aumenta la probabilidad de que aparezcan Cristales de Antimateria y Asteroides Metálicos. |
| **6** | **Orbital Capacity (Capacidad en Pantalla)** | 🌌 | Aumenta el límite máximo de objetos celestes simultáneos en el campo. |

---

## 5. Eventos Temporales (Cosmic Surges)
Cada $3\text{ a }5\text{ minutos}$ se activa un evento dinámico aleatorio con temporizador en la parte superior:
* **Meteor Storm (2:00 min):** Llueven meteoritos a doble velocidad ($\times 2\text{ Masa}, \times 2\text{ Spawn Rate}$).
* **Antimatter Rupture (1:30 min):** Todos los objetos tienen probabilidad de transformarse en antimateria.
* **Relativistic Jet Surge (1:00 min):** El agujero negro emite jets polares y triplica su fuerza de succión.

---

## 6. Sistema de Prestigio: The Big Crunch (Colapso Cósmico)
* **Condición de Desbloqueo:** Alcanzar $1,000,000\text{ kg}$ ($1,000\text{ t}$).
* **Mecánica:** Reinicia las mejoras básicas a cambio de **Primordial Dark Matter Shards**.
* **Efecto Permanente:**
  * Multiplicador global de masa permanente ($\times 1.00 \to \times 2.50 \to \times 10.00 \dots$).
  * Desbloquea el árbol de tecnologías permanentes (*Kerr Spin*, *Hawking Extraction*, *Wormholes*).

---

## 7. Pestañas de la Interfaz
1. **Upgrades:** Panel principal con las 6 mejoras directas de succión, tamaño, valor y spawn.
2. **Missions:** Desafíos contextuales con recompensas de masa y gemas cósmicas.
3. **Milestones:** Logros por hitos de masa ($10\text{ kg}, 100\text{ kg}, 1\text{ t}, 1\text{ M}_\odot$).
4. **Prestige (Big Crunch):** Panel de colapso con vista previa de shards ganados y árbol primordial.
