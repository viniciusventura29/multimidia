// Procurar o adaptador, dos dois jeitos ao mesmo tempo.
//
// Clássico e BLE são duas buscas diferentes do Android, e um adaptador só aparece
// na sua. Um "Bluetooth 4.0" nunca sai na busca clássica — é por isso que ele
// some da tela de pareamento da central e o dono acha que o aparelho está
// quebrado. Um dual-mode aparece nas duas, e aí o clássico ganha: SPP é mais
// estável que GATT com o ELM327.

package com.eclipseos.obdbt

import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.util.Log
import java.util.concurrent.ConcurrentHashMap

internal const val TIPO_SPP = "spp"
internal const val TIPO_BLE = "ble"

/** Um adaptador visto agora, ou já pareado. */
internal data class Achado(
    val nome: String,
    val mac: String,
    val tipo: String,
    val pareado: Boolean,
    val rssi: Int?,
)

/**
 * A busca em curso.
 *
 * Os achados vivem num mapa que o Rust lê por polling (`scanResults`). É polling
 * de propósito: a ponte com o plugin é pedido/resposta, e um canal de eventos só
 * para isto custaria mais do que ler uma lista de dez itens duas vezes por segundo.
 */
internal class Busca(private val context: Context) {
    private val achados = ConcurrentHashMap<String, Achado>()

    @Volatile
    private var comecouEm = 0L
    private var receptor: BroadcastReceiver? = null
    private var scanner: ScanCallback? = null

    @Volatile
    var ativa = false
        private set

    fun iniciar(adapter: BluetoothAdapter) {
        if (ativa) return
        achados.clear()
        comecouEm = System.currentTimeMillis()
        ativa = true

        // Quem já está pareado entra na lista de cara: é quase sempre o adaptador
        // certo, e esperar 12s de varredura para mostrá-lo seria mentir que sumiu.
        for (d in adapter.bondedDevices.orEmpty()) {
            registrar(d, null)
        }

        val filtro = IntentFilter().apply {
            addAction(BluetoothDevice.ACTION_FOUND)
            addAction(BluetoothAdapter.ACTION_DISCOVERY_FINISHED)
        }
        val r = object : BroadcastReceiver() {
            override fun onReceive(ctx: Context, intent: Intent) {
                when (intent.action) {
                    BluetoothDevice.ACTION_FOUND -> {
                        val d = dispositivo(intent) ?: return
                        val rssi = intent.getShortExtra(
                            BluetoothDevice.EXTRA_RSSI,
                            Short.MIN_VALUE,
                        ).toInt()
                        registrar(d, if (rssi == Short.MIN_VALUE.toInt()) null else rssi)
                    }
                    // A busca clássica dura ~12s e para sozinha. Recomeça enquanto
                    // a tela estiver aberta: o adaptador só responde quando o
                    // motorista pluga na tomada OBD, que costuma ser depois.
                    BluetoothAdapter.ACTION_DISCOVERY_FINISHED -> if (ativa) {
                        if (System.currentTimeMillis() - comecouEm > LIMITE_MS) {
                            // Rede de segurança: se ninguém mandou parar (a tela
                            // fechou sem avisar), a busca não fica varrendo para
                            // sempre bloqueando a conexão do carro.
                            Log.i(TAG, "busca passou de ${LIMITE_MS / 1000}s; encerrando sozinha")
                            parar(adapter)
                        } else {
                            try { adapter.startDiscovery() } catch (e: SecurityException) {
                                Log.w(TAG, "sem permissão para recomeçar a busca: ${e.message}")
                            }
                        }
                    }
                }
            }
        }
        registrarReceptor(context, r, filtro)
        receptor = r

        try {
            adapter.cancelDiscovery()
            if (!adapter.startDiscovery()) Log.w(TAG, "startDiscovery devolveu false")
        } catch (e: SecurityException) {
            Log.w(TAG, "sem permissão para a busca clássica: ${e.message}")
        }

        val le = adapter.bluetoothLeScanner
        if (le == null) {
            Log.w(TAG, "esta central não tem scanner BLE")
        } else {
            val cb = object : ScanCallback() {
                override fun onScanResult(tipo: Int, resultado: ScanResult) {
                    registrar(resultado.device, resultado.rssi, resultado.scanRecord?.deviceName)
                }

                override fun onBatchScanResults(resultados: MutableList<ScanResult>) {
                    for (r2 in resultados) {
                        registrar(r2.device, r2.rssi, r2.scanRecord?.deviceName)
                    }
                }

                override fun onScanFailed(erro: Int) {
                    Log.w(TAG, "busca BLE falhou (código $erro)")
                }
            }
            val settings = ScanSettings.Builder()
                .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
                .build()
            try {
                // SEM FILTRO DE UUID de propósito: clone de ELM327 não anuncia o
                // serviço no advertisement, e um filtro o apagaria da lista.
                le.startScan(null, settings, cb)
                scanner = cb
            } catch (e: SecurityException) {
                Log.w(TAG, "sem permissão para a busca BLE: ${e.message}")
            }
        }
        Log.i(TAG, "busca iniciada (clássica + BLE)")
    }

    fun parar(adapter: BluetoothAdapter) {
        ativa = false
        receptor?.let {
            try { context.unregisterReceiver(it) } catch (_: Exception) {}
        }
        receptor = null
        try { adapter.cancelDiscovery() } catch (_: Exception) {}
        scanner?.let {
            try { adapter.bluetoothLeScanner?.stopScan(it) } catch (_: Exception) {}
        }
        scanner = null
        Log.i(TAG, "busca encerrada com ${achados.size} achado(s)")
    }

    /** O sinal mais forte primeiro: é quase sempre o que está plugado no carro. */
    fun resultados(): List<Achado> =
        achados.values.sortedWith(
            compareByDescending<Achado> { it.pareado }.thenByDescending { it.rssi ?: -999 },
        )

    private fun dispositivo(intent: Intent): BluetoothDevice? {
        @Suppress("DEPRECATION")
        return intent.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE)
    }

    private companion object {
        /** Teto de uma busca sem ninguém pedindo para continuar. */
        const val LIMITE_MS = 120_000L
    }

    private fun registrar(d: BluetoothDevice, rssi: Int?, nomeAnunciado: String? = null) {
        val nome = try {
            d.name ?: nomeAnunciado ?: ""
        } catch (e: SecurityException) {
            nomeAnunciado ?: ""
        }
        val pareado = try {
            d.bondState == BluetoothDevice.BOND_BONDED
        } catch (e: SecurityException) {
            false
        }
        val tipo = try {
            if (d.type == BluetoothDevice.DEVICE_TYPE_LE) TIPO_BLE else TIPO_SPP
        } catch (e: SecurityException) {
            TIPO_SPP
        }

        val novo = achados.compute(d.address) { _, antigo ->
            Achado(
                nome = nome.ifEmpty { antigo?.nome ?: "" },
                mac = d.address,
                // Um dual-mode chega pelas duas buscas. Se já foi visto como
                // clássico, continua clássico: SPP é o caminho estável.
                tipo = if (antigo?.tipo == TIPO_SPP) TIPO_SPP else tipo,
                pareado = pareado || (antigo?.pareado ?: false),
                rssi = rssi ?: antigo?.rssi,
            )
        }
        if (novo != null) Log.d(TAG, "achado: \"${novo.nome}\" ${novo.mac} ${novo.tipo}")
    }
}
