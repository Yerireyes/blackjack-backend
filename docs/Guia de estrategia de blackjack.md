# Guía de estrategia de blackjack

**Por qué conviene cada decisión y de dónde salen los porcentajes**

Esta guía es para jugar en cualquier mesa, sin la aplicación. No pretende que memorices
números. Lo que busca es que entiendas *por qué* una jugada es mejor que otra, para que
la estrategia tenga sentido y se te quede.

Todos los porcentajes salen del motor de Blackjack Guide, que calcula cada situación
de forma exacta a partir de las cartas que quedan en el zapato. Salvo que se diga otra
cosa, las reglas son las más comunes en casino:

- 6 mazos.
- El dealer se planta con 17 blando (S17).
- Se puede doblar con cualquier par de cartas, también después de dividir.
- El dealer revisa si tiene blackjack cuando muestra As o 10 (*peek*).
- Hay rendición tardía.
- El blackjack paga 3 a 2.

---

## 1. La idea que lo explica todo: el valor esperado

En blackjack no puedes saber si vas a ganar *esta* mano. Lo que sí puedes saber es
cuánto ganas o pierdes **en promedio** con cada decisión si repites esa misma
situación miles de veces. Ese promedio se llama **valor esperado** (EV).

En esta guía el EV se expresa **por cada 100 apostados**:

- **+20** quiere decir que, en promedio, ganas 20 por cada 100 que apuestas.
- **−54** quiere decir que, en promedio, pierdes 54 por cada 100.

La regla de oro es simple: **elige siempre la acción con el EV más alto**, aunque
todas sean negativas.

Esto último es clave. Muchas manos son malas hagas lo que hagas, y entonces la
pregunta no es "¿cómo gano?" sino "**¿cómo pierdo menos?**". El ejemplo típico es
16 contra un 10 del dealer:

| Acción | Ganas | Empatas | Pierdes | EV por cada 100 |
| --- | --- | --- | --- | --- |
| Plantarse | 23,0 % | 0,0 % | 77,0 % | −54,1 |
| Pedir | 20,3 % | 6,0 % | 73,7 % | −53,5 |
| Rendirse | — | — | — | **−50,0** |

Ninguna opción es buena, pero rendirse (recuperar la mitad de la apuesta) es la que
menos cuesta. Si no se permite rendirse, pedir es apenas mejor que plantarse.

> **Ganar más veces no es lo mismo que ganar más dinero.** Al plantarte con 16 contra
> 10 ganas más veces (23 %) que si pides (20,3 %). Aun así, pedir es mejor, porque
> también te da empates (6 %) y reduce las derrotas.

---

## 2. Cómo se calculan los porcentajes

No hace falta saber programar para entender la idea. Son cuatro pasos.

**Paso 1: contar lo que queda.** La probabilidad de que salga una carta es
*(cuántas quedan de ese valor) ÷ (cuántas cartas quedan en total)*. En 6 mazos hay
312 cartas, y 96 de ellas valen 10 (10, J, Q y K). Por eso cerca del 31 % de las
cartas son dieces. Es la carta más común con mucha diferencia, y casi toda la
estrategia gira alrededor de eso.

> **Ejemplo:** tienes 10 + 6 y el dealer muestra un 10. Te pasas si sale 6, 7, 8, 9 o
> 10. Quedan 23 seises, 24 sietes, 24 ochos, 24 nueves y 94 dieces: 189 de 309 cartas,
> o sea **61 %** de pasarte si pides.

**Paso 2: simular al dealer.** El dealer no decide nada: pide hasta llegar a 17 o
más. Como sus reglas son fijas, se pueden recorrer todas las cartas que podría sacar,
multiplicando probabilidades por el camino. Así se obtiene la probabilidad de que
termine en 17, 18, 19, 20, 21 o pasado.

**Paso 3: evaluar cada acción.**

- *Plantarse:* se compara tu total con la distribución del dealer.
- *Pedir:* para cada carta que podrías recibir se calcula la nueva mano, se elige lo
  mejor desde ahí (seguir pidiendo o plantarse) y se promedia según lo probable que
  sea cada carta.
- *Doblar:* igual que pedir, pero recibes una sola carta y la apuesta vale doble.
- *Dividir:* se juegan dos manos, cada una empezando con una de tus cartas.

**Paso 4: tener en cuenta lo que ya sabes.** Si el dealer muestra As o 10 y revisó
que no tiene blackjack, ya sabes algo de su carta oculta: con un 10 visible, la oculta
**no** es un As. El cálculo se ajusta a eso.

El motor no usa tablas precalculadas. Todo sale de las cartas que quedan, por eso
puede detectar cuándo la composición exacta de tu mano cambia la decisión (sección 9).

---

## 3. Por qué la casa tiene ventaja

Si el dealer y tú siguieran exactamente las mismas reglas, todo parecería parejo.
Pero hay una asimetría que decide todo: **tú juegas primero**. Si te pasas, pierdes
en ese momento, aunque después el dealer también se pase.

Para compensar, el jugador tiene herramientas que el dealer no tiene: plantarse con
menos de 17, doblar, dividir, rendirse y cobrar 3 a 2 por el blackjack. Bien usadas,
dejan la ventaja de la casa en alrededor del **0,5 %**. Jugar "por intuición" o
"imitando al dealer" la multiplica varias veces.

---

## 4. La carta visible del dealer es la mitad de la decisión

Esta tabla muestra cómo suele terminar el dealer según su carta visible:

| Visible | Se pasa | 17 | 18 | 19 | 20 | 21 |
| --- | --- | --- | --- | --- | --- | --- |
| 2 | 35,4 % | 14,0 % | 13,4 % | 13,0 % | 12,4 % | 11,8 % |
| 3 | 37,4 % | 13,4 % | 13,1 % | 12,5 % | 12,1 % | 11,5 % |
| 4 | 39,6 % | 13,1 % | 12,4 % | 12,1 % | 11,6 % | 11,2 % |
| **5** | **41,8 %** | 12,2 % | 12,2 % | 11,8 % | 11,2 % | 10,8 % |
| **6** | **42,3 %** | 16,6 % | 10,6 % | 10,6 % | 10,2 % | 9,7 % |
| 7 | 26,2 % | 36,9 % | 13,8 % | 7,8 % | 7,9 % | 7,4 % |
| 8 | 24,4 % | 12,9 % | 36,0 % | 12,9 % | 6,9 % | 6,9 % |
| 9 | 22,9 % | 12,0 % | 11,7 % | 35,2 % | 12,0 % | 6,1 % |
| 10 | 23,0 % | 12,1 % | 12,1 % | 12,1 % | 36,8 % | 3,8 % |
| A | 16,7 % | 18,8 % | 18,9 % | 18,9 % | 18,9 % | 7,7 % |

*Con As o 10 visible, la tabla supone que el dealer ya revisó y no tiene blackjack.*

**Cartas débiles (2 a 6).** Con un 4, 5 o 6, el valor más probable de la carta oculta
es un 10. Eso deja al dealer en 14, 15 o 16, que es la peor zona posible: está
obligado a pedir y casi cualquier carta grande lo pasa. Por eso el 5 y el 6 se pasan
más del 40 % de las veces.

**Cartas fuertes (7 a As).** Con un 7, 8, 9 o 10, si la oculta es un 10 el dealer ya
tiene 17 a 20 y se planta sin riesgo. Fíjate en la diagonal: con 7 termina sobre
todo en 17, con 8 en 18, con 9 en 19 y con 10 en 20.

> **Mito: "asume que la carta oculta es un 10".** Solo el 31 % de las cartas son
> dieces, así que la mayoría de las veces la oculta *no* es un 10. Es una buena
> intuición para entender la tabla, pero los porcentajes reales son los de arriba.

---

## 5. Manos duras: ¿pedir o plantarse?

Una mano es **dura** cuando no tiene As, o tiene un As que solo puede valer 1.

**Con 11 o menos, nunca te puedes pasar.** Siempre conviene pedir (o doblar, ver
sección 6). **Con 17 o más**, plantarse siempre.

La zona difícil es **12 a 16**. Ahí se enfrentan dos riesgos:

- Si pides, puedes pasarte: 31 % con 12, 39 % con 13, 46 % con 14, 54 % con 15 y
  61 % con 16.
- Si te plantas, solo ganas si el dealer se pasa, porque cualquier total final del
  dealer (17 o más) le gana a tu 12–16.

### Contra 2 a 6: déjalo que se pase

Si te plantas con 12–16, ganas exactamente cuando el dealer se pasa, y contra cartas
débiles eso pasa casi la mitad de las veces. No vale la pena arriesgarte a pasarte
tú primero.

- **16 contra 6:** plantarse **−15,8** vs pedir −42,5. Pedir es casi tres veces peor.
- **13 contra 2:** plantarse **−29,6** vs pedir −30,7. Muy parejo, pero se planta.

**La excepción es el 12 contra 2 y 3.** Con 12 solo te pasas si sale un 10 (31 %), y
el 2 y el 3 son las cartas débiles que menos se pasan. Ahí conviene pedir:

- **12 contra 2:** pedir **−25,2** vs plantarse −29,6.
- **12 contra 3:** pedir **−23,1** vs plantarse −25,5.
- **12 contra 6:** plantarse **−15,5** vs pedir −16,9.

### Contra 7 a As: tienes que mejorar

Contra una carta fuerte, plantarte con 12–16 es esperar a que el dealer se pase, y
eso solo pasa una de cada cuatro veces. Hay que arriesgarse a pedir.

- **16 contra 7:** pedir **−40,9** vs plantarse −47,6. Te pasas el 61 % de las veces
  y aun así conviene pedir.
- **12 contra 7:** pedir **−21,3** vs plantarse −47,5. Aquí la diferencia es enorme.

---

## 6. Doblar: aprovechar cuando vas por delante

Doblar es poner otra apuesta igual a la primera a cambio de recibir **una sola
carta**. Conviene cuando la mano ya es favorable: si vas a ganar más de lo que
pierdes, quieres tener más dinero en la mesa.

**Con 11** puedes hacer 21 con cualquier 10 (31 % de las cartas), y no te puedes
pasar. **Con 10** pasa algo parecido. **Con 9** la ventaja es menor, así que solo se
dobla contra cartas débiles.

| Mano | Contra | Pedir | Doblar | Decisión |
| --- | --- | --- | --- | --- |
| 11 | 6 | +34,1 | **+68,3** | Doblar |
| 11 | 10 | +11,9 | **+17,8** | Doblar |
| 11 | As | **+14,8** | +13,0 | Pedir |
| 10 | 9 | +11,7 | **+14,8** | Doblar |
| 10 | 10 | **+2,7** | −0,6 | Pedir |
| 9 | 3 | +10,8 | **+13,3** | Doblar |
| 9 | 7 | **+17,6** | +11,8 | Pedir |

¿Por qué no siempre doblar con 11? Al doblar renuncias a pedir más cartas. Contra un
As o un 10, el dealer termina con totales altos muy seguido. Si recibes una carta
chica, te quedas atascado en 13 o 14 con el doble de dinero en juego. Pidiendo
normal, puedes seguir mejorando la mano.

---

## 7. Manos blandas: el As que te protege

Una mano es **blanda** cuando tiene un As que vale 11, como A+6 = 17 blando. La
ventaja es que **no te puedes pasar con la siguiente carta**: si se pasa de 21, el As
vuelve a valer 1.

Por eso las manos blandas se juegan de forma más agresiva:

- **A+2 a A+6:** son manos débiles (13 a 17) que no se pueden romper. Contra cartas
  débiles se dobla, contra fuertes se pide. **Nunca te plantes con 17 blando**: contra
  un 7 lo más probable es que el dealer haga 17 o más. A+6 contra 7: pedir **+5,5**
  vs plantarse −10,4.
- **A+7 (18 blando) es la mano más mal jugada.** Parece buena, pero depende de la
  carta del dealer:
  - Contra 3–6: **doblar** (contra 6: +38,3 vs plantarse +28,0).
  - Contra 2, 7 y 8: **plantarse** (contra 7 el 18 le gana al 17 que más se repite).
  - Contra 9, 10 y As: **pedir** (contra 9: pedir −9,8 vs plantarse −18,3). Un 18
    pierde mucho contra el 19 y el 20 que esas cartas suelen hacer.
- **A+8 y A+9:** plantarse siempre. Ya son 19 y 20.

---

## 8. Dividir: cuándo y por qué

Dividir es separar un par en dos manos, poniendo una segunda apuesta igual. Conviene
por una de tres razones.

**1. Convertir una mano muy mala en dos regulares.** El caso clásico es **8+8**: un
16, el peor total que existe. Dividido, cada mano empieza con 8, que es un comienzo
aceptable.

- **8+8 contra 10:** dividir **−47,5**, rendirse −50,0, plantarse −53,7. Sigue
  siendo negativo, pero es lo que menos pierde.
- **8+8 contra 6:** dividir **+40,6** vs plantarse −15,7. Pasas de perder a ganar.

**2. Convertir un buen comienzo en dos buenos comienzos.** **A+A** es un 12 (o 2), una
mano mediocre. Dividido, cada mano empieza con un As, la mejor carta del juego.

- **A+A contra 6:** dividir **+68,2** vs pedir +18,8.
- **A+A contra 10:** dividir **+18,2** vs pedir −6,6.

**Por eso A+A y 8+8 se dividen siempre.**

**3. Aprovechar a un dealer débil.** Pares chicos como **2+2, 3+3, 6+6 y 7+7** se
dividen contra cartas débiles. Cada mano parte de una situación pobre, pero el dealer
se va a pasar seguido, y ahí quieres tener dos apuestas en la mesa. Que se pueda doblar
después de dividir ayuda mucho.

- **6+6 contra 6:** dividir **+15,8** vs plantarse −15,5.
- **7+7 contra 10:** pedir **−47,4** vs dividir −65,1. Contra un dealer fuerte, dos
  manos malas son dos apuestas perdedoras.

### Lo que nunca se divide

- **10+10:** ya tienes 20, que gana casi siempre. Contra 6: plantarse **+70,3** vs
  dividir +45,5. Romper un 20 es regalar dinero.
- **5+5:** juntos son un 10, un total excelente para doblar. Separados son dos manos
  que empiezan con 5, uno de los peores comienzos. Contra 6: doblar **+59,9** vs
  dividir +6,1.
- **4+4:** es un 8. Solo se divide contra 5 y 6, y solo si se permite doblar después
  de dividir.

### El caso curioso del 9+9

El 9+9 (18) se divide contra 2–6, 8 y 9, pero **no contra 7**. Contra un 7, el dealer
suele terminar en 17, y tu 18 le gana: plantarse **+40,0** vs dividir +36,4. Contra 8
o 9 el dealer tiende a hacer 18 o 19, tu 18 empata o pierde, y conviene dividir.

---

## 9. Cuando las cartas exactas importan

La estrategia básica habla de *totales*: "16 contra 10". Pero dos manos con el mismo
total no siempre son iguales, porque **las cartas que tienes ya no están en el
zapato**.

**16 contra 10 en un solo mazo:**

| Tu mano | Pedir | Plantarse | Por qué |
| --- | --- | --- | --- |
| 10 + 6 | **−50,7** | −54,3 | Te quedan más cartas chicas para mejorar |
| 4 + 4 + 4 + 4 | −62,8 | **−56,5** | Ya usaste cuatro 4: quedan pocas cartas chicas |

**12 contra 4 (6 mazos):** la tabla clásica dice plantarse, pero con 10 + 2 el motor
da pedir −21,0 vs plantarse −21,1. Es prácticamente un empate: cualquiera de las dos
está bien.

Estas diferencias son chicas y casi siempre pesan más con pocos mazos. En una mesa de
6 u 8 mazos, la tabla del final es más que suficiente.

---

## 10. Rendirse y el seguro

### Rendirse

Rendirse es abandonar la mano y recuperar la mitad de la apuesta: **siempre pierdes
50**. Por eso solo conviene cuando cualquier otra acción pierde *más* de 50 por cada
100. Con estas reglas pasa con:

- **16 contra 9, 10 y As.** Contra As: plantarse −66,5, pedir −51,6, rendirse
  **−50,0**.
- **15 contra 10.**

No te rindas con 8+8 (es mejor dividir) ni con 17 contra As (plantarse −47,6 es mejor
que −50,0).

### El seguro: no lo tomes

Cuando el dealer muestra un As, te ofrece un "seguro": apuestas la mitad de tu apuesta
a que tiene blackjack, y si acierta te pagan 2 a 1.

Para que valga la pena, el dealer tendría que tener blackjack **más de 1 de cada 3
veces** (33,3 %). En 6 mazos, la probabilidad de que la carta oculta sea un 10 es
**30,9 %**. Eso hace que el seguro pierda alrededor de **7 por cada 100** que apuestas
en él.

Tampoco lo tomes cuando tienes blackjack (el "dinero parejo"): es exactamente la misma
apuesta con otro nombre.

---

## 11. Las reglas de la mesa cambian la estrategia

Antes de sentarte, fíjate en las reglas. Varias cambian decisiones concretas o el
dinero que pierdes a la larga.

- **El blackjack paga 6 a 5 en lugar de 3 a 2.** Es la regla más cara: aumenta la
  ventaja de la casa en alrededor de 1,4 puntos, más que todas las demás juntas.
  **Evita estas mesas.**
- **El dealer pide con 17 blando (H17).** Lo hace un poco más fuerte. Cambia jugadas
  como A+7 contra 2, que pasa de plantarse a doblar.
- **El dealer no revisa si tiene blackjack (sin *peek*, típico en Europa).** Si
  doblas o divides y el dealer tiene blackjack, pierdes todo lo apostado. Por eso se
  dobla y se divide menos contra As y 10. Por ejemplo, 11 contra As pasa de +14,8 a
  −20,9.
- **No se permite rendirse.** 16 contra 10 pasa a ser pedir, y 8+8 contra 10 se sigue
  dividiendo.
- **No se puede doblar después de dividir.** Se dividen menos pares chicos (2+2, 3+3,
  4+4 y 6+6) contra cartas débiles.

---

## 12. Tabla de estrategia básica

**Reglas:** 6 mazos, S17, doblar con cualquier par de cartas, doblar después de
dividir, *peek* y rendición tardía. La tabla la generó el motor con la composición
más común de cada total (por ejemplo, 12 = 10 + 2).

<div class="leyenda">
<span class="stand">S</span> Plantarse
<span class="hit">P</span> Pedir
<span class="double">D</span> Doblar (Dp: si no se puede, pedir · Ds: si no se puede, plantarse)
<span class="split">Div</span> Dividir
<span class="surrender">R</span> Rendirse (Rp: si no se puede, pedir)
</div>

### Manos duras

<table class="estrategia"><thead><tr><th>Dura</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>10</th><th>A</th></tr></thead><tbody><tr><th>5</th><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>6</th><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>7</th><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>8</th><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>9</th><td class="hit">P</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>10</th><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>11</th><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="hit">P</td></tr><tr><th>12</th><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="stand">S</td><td class="stand">S</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>13</th><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>14</th><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>15</th><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="surrender">Rp</td><td class="hit">P</td></tr><tr><th>16</th><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="hit">P</td><td class="hit">P</td><td class="surrender">Rp</td><td class="surrender">Rp</td><td class="surrender">Rp</td></tr><tr><th>17</th><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td></tr></tbody></table>

### Manos blandas

<table class="estrategia"><thead><tr><th>Blanda</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>10</th><th>A</th></tr></thead><tbody><tr><th>A,2</th><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="double">Dp</td><td class="double">Dp</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>A,3</th><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="double">Dp</td><td class="double">Dp</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>A,4</th><td class="hit">P</td><td class="hit">P</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>A,5</th><td class="hit">P</td><td class="hit">P</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>A,6</th><td class="hit">P</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>A,7</th><td class="stand">S</td><td class="double">Ds</td><td class="double">Ds</td><td class="double">Ds</td><td class="double">Ds</td><td class="stand">S</td><td class="stand">S</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>A,8</th><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td></tr><tr><th>A,9</th><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td></tr></tbody></table>

### Pares

<table class="estrategia"><thead><tr><th>Par</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>10</th><th>A</th></tr></thead><tbody><tr><th>A,A</th><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td></tr><tr><th>2,2</th><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>3,3</th><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>4,4</th><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="split">Div</td><td class="split">Div</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>5,5</th><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="double">Dp</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>6,6</th><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>7,7</th><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td><td class="hit">P</td></tr><tr><th>8,8</th><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td></tr><tr><th>9,9</th><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="split">Div</td><td class="stand">S</td><td class="split">Div</td><td class="split">Div</td><td class="stand">S</td><td class="stand">S</td></tr><tr><th>10,10</th><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td><td class="stand">S</td></tr></tbody></table>

### Cinco reglas para recordar si olvidas la tabla

1. Con 17 duro o más, plántate. Con 11 o menos, pide o dobla.
2. Con 12–16: plántate contra 2–6 y pide contra 7–As. La excepción es 12 contra 2 y 3,
   donde se pide.
3. Divide siempre A+A y 8+8. Nunca dividas 10+10 ni 5+5.
4. A+7 no es una mano hecha: dobla contra 3–6 y pide contra 9, 10 y As.
5. No tomes el seguro.
