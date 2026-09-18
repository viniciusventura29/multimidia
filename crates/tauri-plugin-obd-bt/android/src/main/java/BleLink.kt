// Bluetooth Low Energy: o mesmo ELM327, falado por GATT.
//
// Metade dos adaptadores vendidos hoje (os "Bluetooth 4.0", tipo o iCar Pro BLE)
// não fala SPP — e nem aparece na tela de pareamento do Android, porque BLE não
// pareia: o app conecta direto. Aqui não há socket nem stream; há uma
// característica em que se escreve o comando e outra que notifica a resposta em
// pedaços, que este arquivo remonta até o prompt '>'.
//
// Armadilhas que custam horas, todas tratadas abaixo:
//   - sem TRANSPORT_LE, um dongle dual-mode negocia BR/EDR e a descoberta volta vazia;
//   - habilitar notificação SEM escrever o descritor CCCD conecta, não dá erro, e
//     nenhum byte chega nunca;
//   - GATT é uma operação por vez: escrever sem esperar o callback da anterior
//     perde a escrita calado.

package com.eclipseos.obdbt

import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothProfile
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import java.util.UUID
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.CountDownLatch
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

internal class BleLink private constructor(
    private val gatt: BluetoothGatt,
    private val estado: Estado,
    private val escreve: BluetoothGattCharacteristic,
) : Link {

    override fun send(cmd: String, timeoutMs: Int): String {
        estado.erroFatal()?.let { throw ObdBtFalha(it) }

        // Resto da resposta anterior (o carro respondeu depois do timeout) não pode
        // contaminar a próxima pergunta: o painel passaria a mostrar sempre a
        // leitura de trás.
        estado.chegando.clear()

        val bytes = (cmd + "\r").toByteArray(Charsets.US_ASCII)
        val pedaco = (estado.mtu - 3).coerceAtLeast(20)
        var i = 0
        while (i < bytes.size) {
            val fim = minOf(i + pedaco, bytes.size)
            escrever(bytes.copyOfRange(i, fim))
            i = fim
        }

        val coletor = Coletor()
        val limite = System.currentTimeMillis() + timeoutMs
        var achouPrompt = false
        while (true) {
            val resta = limite - System.currentTimeMillis()
            if (resta <= 0) break
            val parte = estado.chegando.poll(resta, TimeUnit.MILLISECONDS) ?: break
            if (coletor.push(parte, parte.size)) {
                achouPrompt = true
                break
            }
        }
        estado.erroFatal()?.let { throw ObdBtFalha(it) }
        return resposta(coletor, cmd, timeoutMs, achouPrompt)
    }

    /** Uma fatia, esperando o callback antes da próxima — GATT não enfileira. */
    private fun escrever(fatia: ByteArray) {
        estado.escritaPronta.clear()
        val tipo = if (escreve.properties and BluetoothGattCharacteristic.PROPERTY_WRITE != 0) {
            BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
        } else {
            BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
        }

        val enfileirou = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            gatt.writeCharacteristic(escreve, fatia, tipo) == BluetoothGatt.GATT_SUCCESS
        } else {
            @Suppress("DEPRECATION")
            run {
                escreve.writeType = tipo
                escreve.value = fatia
                gatt.writeCharacteristic(escreve)
            }
        }
        if (!enfileirou) throw ObdBtFalha("GATT recusou a escrita no adaptador")

        val status = estado.escritaPronta.poll(ESPERA_ESCRITA_MS, TimeUnit.MILLISECONDS)
            ?: throw ObdBtFalha("o adaptador não confirmou a escrita")
        if (status != BluetoothGatt.GATT_SUCCESS) {
            throw ObdBtFalha("escrita no adaptador falhou (status $status)")
        }
    }

    override fun close() {
        try { gatt.disconnect() } catch (_: Exception) {}
        try { gatt.close() } catch (_: Exception) {}
    }

    /** O que o callback do GATT (em outra thread) entrega para o `send`. */
    private class Estado {
        val conectou = CountDownLatch(1)
        val mtuPronto = CountDownLatch(1)
        val servicos = CountDownLatch(1)
        val descritorPronto = CountDownLatch(1)
        val escritaPronta = ArrayBlockingQueue<Int>(1)
        val chegando = LinkedBlockingQueue<ByteArray>()

        @Volatile var mtu = 23
        @Volatile var caiu = false
        @Volatile var falha: String? = null

        /** A razão para desistir da conexão, se houver. */
        fun erroFatal(): String? = falha ?: if (caiu) "o adaptador desconectou" else null

        fun morreu(razao: String) {
            falha = razao
            caiu = true
            // Solta quem estiver esperando, em vez de deixar estourar o timeout.
            conectou.countDown()
            mtuPronto.countDown()
            servicos.countDown()
            descritorPronto.countDown()
            escritaPronta.offer(BluetoothGatt.GATT_FAILURE)
        }
    }

    companion object {
        private const val ESPERA_ESCRITA_MS = 3_000L

        /** Client Characteristic Configuration — o descritor que liga a notificação. */
        private val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805F9B34FB")

        /** Serviços do próprio Bluetooth: nunca carregam o canal do ELM327. */
        private val SERVICOS_DO_SISTEMA: Set<UUID> = setOf(
            curto("1800"), // Generic Access
            curto("1801"), // Generic Attribute
            curto("180A"), // Device Information
            curto("180F"), // Battery
            curto("1804"), // Tx Power
        )

        /**
         * Pares conhecidos: serviço -> (quem notifica, quem recebe a escrita).
         *
         * Tentados primeiro porque são certeza. A heurística abaixo cobre o resto —
         * e cobre bem, porque firmware de clone inventa UUID mas não inventa
         * propriedade: quem notifica notifica, quem escreve escreve.
         */
        private val CONHECIDOS: List<Triple<UUID, UUID, UUID>> = listOf(
            // Vgate iCar Pro BLE / Viecar / vários clones.
            Triple(curto("FFF0"), curto("FFF1"), curto("FFF2")),
            // Módulo HM-10 (LELink e parentes): uma característica só, nos dois papéis.
            Triple(curto("FFE0"), curto("FFE1"), curto("FFE1")),
            // Konnwei e outros.
            Triple(curto("18F0"), curto("2AF0"), curto("2AF1")),
            // Vgate iCar Pro BLE pelo perfil "de iPhone".
            Triple(
                UUID.fromString("E7810A71-73AE-499D-8C15-FAA9AEF0C3F2"),
                UUID.fromString("BEF8D6C9-9C21-4C9E-B632-BD58C1009F9F"),
                UUID.fromString("BEF8D6C9-9C21-4C9E-B632-BD58C1009F9F"),
            ),
        )

        private fun curto(hex: String): UUID =
            UUID.fromString("0000${hex.lowercase()}-0000-1000-8000-00805f9b34fb")

        fun abrir(context: Context, device: BluetoothDevice, timeoutMs: Long): BleLink {
            val estado = Estado()
            val principal = Handler(Looper.getMainLooper())

            val callback = object : BluetoothGattCallback() {
                override fun onConnectionStateChange(g: BluetoothGatt, status: Int, novo: Int) {
                    when (novo) {
                        BluetoothProfile.STATE_CONNECTED -> {
                            Log.i(TAG, "GATT conectado; pedindo MTU")
                            estado.conectou.countDown()
                            // Fora da thread do callback: algumas pilhas ignoram
                            // uma operação disparada de dentro dela.
                            principal.postDelayed({
                                if (!g.requestMtu(517)) {
                                    estado.mtuPronto.countDown()
                                    g.discoverServices()
                                }
                            }, 200)
                        }
                        BluetoothProfile.STATE_DISCONNECTED ->
                            estado.morreu("o adaptador desconectou (status $status)")
                    }
                }

                override fun onMtuChanged(g: BluetoothGatt, mtu: Int, status: Int) {
                    if (status == BluetoothGatt.GATT_SUCCESS) estado.mtu = mtu
                    Log.i(TAG, "MTU = ${estado.mtu}; descobrindo serviços")
                    estado.mtuPronto.countDown()
                    g.discoverServices()
                }

                override fun onServicesDiscovered(g: BluetoothGatt, status: Int) {
                    if (status != BluetoothGatt.GATT_SUCCESS) {
                        estado.morreu("não deu para descobrir os serviços (status $status)")
                        return
                    }
                    estado.servicos.countDown()
                }

                override fun onDescriptorWrite(
                    g: BluetoothGatt,
                    d: BluetoothGattDescriptor,
                    status: Int,
                ) {
                    estado.descritorPronto.countDown()
                }

                override fun onCharacteristicWrite(
                    g: BluetoothGatt,
                    c: BluetoothGattCharacteristic,
                    status: Int,
                ) {
                    estado.escritaPronta.offer(status)
                }

                // API 33+ entrega o valor como argumento...
                override fun onCharacteristicChanged(
                    g: BluetoothGatt,
                    c: BluetoothGattCharacteristic,
                    value: ByteArray,
                ) {
                    estado.chegando.offer(value)
                }

                // ...e abaixo dela, dentro da característica. A head unit é Android
                // 10-12, então este é o caminho que roda no carro.
                @Suppress("DEPRECATION")
                override fun onCharacteristicChanged(
                    g: BluetoothGatt,
                    c: BluetoothGattCharacteristic,
                ) {
                    c.value?.let { estado.chegando.offer(it) }
                }
            }

            val gatt = device.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE)
                ?: throw ObdBtFalha("não deu para abrir o GATT do adaptador")

            try {
                if (!estado.conectou.await(timeoutMs, TimeUnit.MILLISECONDS)) {
                    throw ObdBtFalha("o adaptador BLE não conectou em ${timeoutMs}ms")
                }
                estado.erroFatal()?.let { throw ObdBtFalha(it) }

                // O MTU é otimização: se o adaptador não responder, segue com 23.
                estado.mtuPronto.await(3, TimeUnit.SECONDS)

                if (!estado.servicos.await(timeoutMs, TimeUnit.MILLISECONDS)) {
                    throw ObdBtFalha("o adaptador BLE não listou os serviços")
                }
                estado.erroFatal()?.let { throw ObdBtFalha(it) }

                val (notifica, escreve) = escolherCanal(gatt)
                    ?: throw ObdBtFalha("este dispositivo BLE não parece um adaptador OBD")
                Log.i(TAG, "canal BLE: notifica=${notifica.uuid} escreve=${escreve.uuid}")

                ligarNotificacao(gatt, notifica, estado)
                return BleLink(gatt, estado, escreve)
            } catch (e: Exception) {
                try { gatt.disconnect() } catch (_: Exception) {}
                try { gatt.close() } catch (_: Exception) {}
                throw e
            }
        }

        /**
         * Acha o par (notifica, escreve).
         *
         * Tabela primeiro, propriedade depois. A propriedade é a regra que
         * sobrevive a clone: num serviço que não é do sistema, quem tem NOTIFY (ou
         * INDICATE) é a boca do adaptador, e quem tem WRITE é o ouvido — às vezes a
         * mesma característica, como no HM-10.
         */
        private fun escolherCanal(
            gatt: BluetoothGatt,
        ): Pair<BluetoothGattCharacteristic, BluetoothGattCharacteristic>? {
            for ((servico, leitura, escrita) in CONHECIDOS) {
                val s: BluetoothGattService = gatt.getService(servico) ?: continue
                val n = s.getCharacteristic(leitura) ?: continue
                val w = s.getCharacteristic(escrita) ?: continue
                if (notifica(n) && escrevivel(w)) return n to w
            }

            for (s in gatt.services) {
                if (s.uuid in SERVICOS_DO_SISTEMA) continue
                val n = s.characteristics.firstOrNull { notifica(it) } ?: continue
                val w = s.characteristics.firstOrNull { escrevivel(it) } ?: continue
                return n to w
            }
            return null
        }

        private fun notifica(c: BluetoothGattCharacteristic): Boolean {
            val p = BluetoothGattCharacteristic.PROPERTY_NOTIFY or
                BluetoothGattCharacteristic.PROPERTY_INDICATE
            return c.properties and p != 0
        }

        private fun escrevivel(c: BluetoothGattCharacteristic): Boolean {
            val p = BluetoothGattCharacteristic.PROPERTY_WRITE or
                BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE
            return c.properties and p != 0
        }

        /**
         * Liga a notificação — nas duas pontas.
         *
         * `setCharacteristicNotification` só avisa a pilha do Android; quem manda o
         * adaptador começar a falar é a escrita no descritor CCCD. Fazer só a
         * primeira conecta, não dá erro, e deixa o painel esperando para sempre.
         */
        private fun ligarNotificacao(
            gatt: BluetoothGatt,
            c: BluetoothGattCharacteristic,
            estado: Estado,
        ) {
            if (!gatt.setCharacteristicNotification(c, true)) {
                throw ObdBtFalha("o adaptador recusou ligar a notificação")
            }
            val cccd = c.getDescriptor(CCCD) ?: return
            val valor = if (c.properties and BluetoothGattCharacteristic.PROPERTY_NOTIFY != 0) {
                BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
            } else {
                BluetoothGattDescriptor.ENABLE_INDICATION_VALUE
            }

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                gatt.writeDescriptor(cccd, valor)
            } else {
                @Suppress("DEPRECATION")
                run {
                    cccd.value = valor
                    gatt.writeDescriptor(cccd)
                }
            }
            estado.descritorPronto.await(3, TimeUnit.SECONDS)
        }
    }
}
