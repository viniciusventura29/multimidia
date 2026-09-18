// Parear pelo app, sem depender da tela de Bluetooth do Android.
//
// É o motivo deste arquivo existir: na central UIS7862 o iCar Pro APARECE na
// busca do sistema e não pareia. O app não está preso àquela tela — `createBond`
// fala com a pilha de Bluetooth direto, e o diálogo de PIN pode ser respondido
// por código. É o que Car Scanner e Torque fazem.

package com.eclipseos.obdbt

import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.util.Log
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

internal object Pareamento {
    /**
     * Os PINs de fábrica dos clones de ELM327, na ordem em que aparecem no mundo.
     *
     * Tentar em sequência é o que um dono faria olhando o manual — só que sem
     * manual, porque o adaptador veio numa caixinha sem nada escrito.
     */
    private val PINS = listOf("1234", "0000", "6789", "1111")

    private const val ESPERA_MS = 20_000L

    /**
     * Cria o vínculo com o adaptador, respondendo o PIN.
     *
     * Devolve `true` se terminou pareado. O que NÃO dá para automatizar é a
     * variante por confirmação de passkey: `setPairingConfirmation` exige
     * `BLUETOOTH_PRIVILEGED`, que é de app de sistema. Aí o diálogo do Android
     * aparece e o dono toca em "Parear" — ainda assim é mais do que ele consegue
     * hoje, porque agora existe o diálogo.
     */
    fun parear(context: Context, adapter: BluetoothAdapter, device: BluetoothDevice): Boolean {
        if (bondState(device) == BluetoothDevice.BOND_BONDED) {
            Log.i(TAG, "${device.address} já estava pareado")
            return true
        }

        // Descoberta ativa atrapalha o handshake do pareamento tanto quanto o do
        // RFCOMM.
        try { adapter.cancelDiscovery() } catch (_: SecurityException) {}

        for (pin in PINS) {
            Log.i(TAG, "pareando ${device.address} com PIN $pin")
            if (tentar(context, device, pin)) {
                Log.i(TAG, "pareado: ${device.address}")
                return true
            }
            if (bondState(device) == BluetoothDevice.BOND_BONDED) return true
        }
        Log.w(TAG, "não pareou ${device.address} com nenhum PIN conhecido")
        return false
    }

    private fun tentar(context: Context, device: BluetoothDevice, pin: String): Boolean {
        val acabou = CountDownLatch(1)

        val receptor = object : BroadcastReceiver() {
            override fun onReceive(ctx: Context, intent: Intent) {
                val alvo: BluetoothDevice? = dispositivo(intent)
                if (alvo == null || !alvo.address.equals(device.address, ignoreCase = true)) return

                when (intent.action) {
                    BluetoothDevice.ACTION_PAIRING_REQUEST -> {
                        val variante = intent.getIntExtra(
                            BluetoothDevice.EXTRA_PAIRING_VARIANT,
                            BluetoothDevice.ERROR,
                        )
                        responder(alvo, variante, pin)
                    }
                    BluetoothDevice.ACTION_BOND_STATE_CHANGED -> {
                        val estado = intent.getIntExtra(
                            BluetoothDevice.EXTRA_BOND_STATE,
                            BluetoothDevice.ERROR,
                        )
                        if (estado != BluetoothDevice.BOND_BONDING) acabou.countDown()
                    }
                }
            }
        }

        val filtro = IntentFilter().apply {
            addAction(BluetoothDevice.ACTION_PAIRING_REQUEST)
            addAction(BluetoothDevice.ACTION_BOND_STATE_CHANGED)
        }
        // Registrado ANTES do createBond: o pedido de PIN chega em milissegundos, e
        // registrar depois perde o diálogo para o sistema.
        registrarReceptor(context, receptor, filtro)

        return try {
            if (!device.createBond()) {
                Log.w(TAG, "createBond devolveu false para ${device.address}")
                return false
            }
            acabou.await(ESPERA_MS, TimeUnit.MILLISECONDS)
            bondState(device) == BluetoothDevice.BOND_BONDED
        } catch (e: SecurityException) {
            Log.w(TAG, "sem permissão para parear: ${e.message}")
            false
        } finally {
            try { context.unregisterReceiver(receptor) } catch (_: Exception) {}
        }
    }

    private fun responder(device: BluetoothDevice, variante: Int, pin: String) {
        try {
            when (variante) {
                BluetoothDevice.PAIRING_VARIANT_PIN -> {
                    device.setPin(pin.toByteArray(Charsets.US_ASCII))
                }
                else -> {
                    // Passkey/consentimento: só app de sistema confirma por código.
                    // Tenta mesmo assim — em algumas centrais o app vem assinado com
                    // a plataforma — e, se não der, o diálogo do Android assume.
                    device.setPairingConfirmation(true)
                }
            }
        } catch (e: SecurityException) {
            Log.i(TAG, "confirmação de pareamento ficou com o diálogo do Android")
        } catch (e: Exception) {
            Log.w(TAG, "não consegui responder o pedido de pareamento: ${e.message}")
        }
    }

    private fun bondState(device: BluetoothDevice): Int = try {
        device.bondState
    } catch (e: SecurityException) {
        BluetoothDevice.ERROR
    }

    private fun dispositivo(intent: Intent): BluetoothDevice? {
        @Suppress("DEPRECATION")
        return intent.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE)
    }
}
