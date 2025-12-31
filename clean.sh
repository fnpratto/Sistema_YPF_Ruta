#!/bin/bash
echo "🧹 Limpiando sistema YPF completamente..."

# Matar TODOS los procesos relacionados
echo "Matando procesos cargo..."
sudo pkill -f cargo 2>/dev/null
sudo pkill -f company 2>/dev/null
sudo pkill -f regional_admin 2>/dev/null
sudo pkill -f company_admin 2>/dev/null
sudo pkill -f gas_pump 2>/dev/null
sudo pkill -f card 2>/dev/null

# Esperar que terminen
sleep 3

# Liberar puertos específicos que usa tu test
echo "Liberando puertos específicos..."
for port in 8080 9001 9002 10000 10010 11000 11010 12000 12010 13000 13010; do
    sudo fuser -k ${port}/tcp 2>/dev/null
    sudo fuser -k ${port}/udp 2>/dev/null
done

# Verificar que los puertos estén libres
echo "Verificando puertos liberados..."
for port in 8080 9001 9002 10000 10010 11000 11010; do
    if netstat -tulpn | grep -q ":${port}"; then
        echo "❌ Puerto ${port} aún ocupado"
        sudo lsof -ti:${port} | xargs sudo kill -9 2>/dev/null
    else
        echo "✅ Puerto ${port} libre"
    fi
done

echo "✅ Limpieza completa"


