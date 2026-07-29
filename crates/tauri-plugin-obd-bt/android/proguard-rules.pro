# As classes do plugin são chamadas por reflexão pela runtime do Tauri; não
# deixe o R8 removê-las nem renomeá-las.
-keep class com.eclipseos.obdbt.** { *; }
