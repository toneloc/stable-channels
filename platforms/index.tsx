import React, { useState, useEffect, useRef } from 'react';
import { Image, StyleSheet, Platform, TouchableOpacity, Text, View, Animated } from 'react-native';

import { HelloWave } from '@/components/HelloWave';
import ParallaxScrollView from '@/components/ParallaxScrollView';
import { ThemedText } from '@/components/ThemedText';
import { ThemedView } from '@/components/ThemedView';
import {Builder, Config, ChannelConfig, Node} from 'ldk-node-rn';
import {ChannelDetails, NetAddress, LogLevel} from 'ldk-node-rn/lib/classes/Bindings';
import RNFS from 'react-native-fs';
import {addressToString} from 'ldk-node-rn/lib/utils';

let docDir = RNFS.DocumentDirectoryPath + '/NEW_LDK_NODE/' + `${Platform.Version}/`;
console.log('Platform Version=====>', `${Platform.Version}`);


export default function HomeScreen() {
  const [message, setMessage] = useState('');
  const scaleAnim = useRef(new Animated.Value(1)).current;

  useEffect(() => {
    Animated.loop(
      Animated.sequence([
        Animated.timing(scaleAnim, {
          toValue: 1.05,
          duration: 1000,
          useNativeDriver: true,
        }),
        Animated.timing(scaleAnim, {
          toValue: 1,
          duration: 1000,
          useNativeDriver: true,
        }),
      ])
    ).start();
  }, [scaleAnim]);

  const handlePress = async () => {
    console.log("here");
    const mnemonic = 'absurd aware donate anxiety gather lottery advice document advice choice limb balance';
    
    const buildNode = async (mnemonic: string) => {
        let host;
        let port = 39735;
        let esploraServer;

        host = '0.0.0.0';
        if (Platform.OS === 'android') {
            host = '0.0.0.0';
        } else if (Platform.OS === 'ios') {
            host = '0.0.0.0';
        }

        esploraServer = `https://mutinynet.ltbl.io/api`;
        console.log("here2");
        try {
            let docDir = RNFS.DocumentDirectoryPath + '/NEW_LDK_NODE/' + `${Platform.Version}/`;
            const storagePath = docDir;
            console.log('storagePath====>', storagePath);

            console.log("here3");
            console.log('storagePath====>', storagePath);

            const ldkPort = Platform.OS === 'ios' ? (Platform.Version == '17.0' ? 2000 : 2001) : 8081;
            const config = await new Config().create(storagePath, docDir + 'logs', 'signet', [new NetAddress(host, ldkPort)]);
            const builder = await new Builder().fromConfig(config);
            await builder.setNetwork('signet');
            await builder.setEsploraServer(esploraServer);
            const key = await builder.setEntropyBip39Mnemonic(mnemonic);
            console.log('---Key--- ', key);
            await builder.setLiquiditySourceLsps2('44.219.111.31:39735', '0371d6fd7d75de2d0372d03ea00e8bacdacb50c27d0eaea0a76a0622eff1f5ef2b', 'JZWN9YLW');

            const nodeObj: Node = await builder.build();

            const started = await nodeObj.start();
            if (started) {
                console.log('Node started successfully');
            } else {
                console.log('Node failed to start');
            }

            const nodeId = await nodeObj.nodeId();
            const listeningAddr = await nodeObj.listeningAddresses();
            console.log('Node Info:', { nodeId: nodeId.keyHex, listeningAddress: `${listeningAddr?.map(i => addressToString(i))}` });
        } catch (e) {
            console.error('Error in starting and building Node:', e);
        }
    };

    await buildNode(mnemonic);
};
  return (
    <ParallaxScrollView
      headerBackgroundColor={{ light: '#ffffff', dark: '#ffffff' }}
      headerImage={
        <Image
          source={require('@/assets/images/partial-react-logo.png')}
          style={styles.reactLogo}
        />
      }>
      <ThemedView style={styles.titleContainer}>
        <ThemedText type="title">Stablecorn</ThemedText>
        <HelloWave />
      </ThemedView>
      <ThemedView style={styles.stepContainer}>
        <ThemedText type="subtitle">Step 1: Get a Lightning invoice ⚡</ThemedText>
        <ThemedText>
          Press the "Stabilize" button below. 
        </ThemedText>
      </ThemedView>
      <ThemedView style={styles.stepContainer}>
        <ThemedText type="subtitle">Step 2: Send yourself bitcoin. 💸</ThemedText>
        <ThemedText>
        You can do this from another app or your account on Coinbase or Binance.       
        </ThemedText>
      </ThemedView>
      <ThemedView style={styles.stepContainer}>
        <ThemedText type="subtitle">Step 3: Stability activated 🔧</ThemedText>
        <ThemedText>
         Your keys, your bitcoin, your tools.
        </ThemedText>
      </ThemedView>
      <View style={styles.buttonContainer}>
        <Animated.View style={{ transform: [{ scale: scaleAnim }] }}>
          <TouchableOpacity style={styles.button} onPress={() => handlePress()}>
            <Text style={styles.buttonText}>Stabilize</Text>
          </TouchableOpacity>
        </Animated.View>
      </View>
      {message !== '' && (
        <ThemedView style={styles.messageContainer}>
          <ThemedText>{message}</ThemedText>
        </ThemedView>
      )}
    </ParallaxScrollView>
  );
}

const styles = StyleSheet.create({
  titleContainer: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
  },
  stepContainer: {
    gap: 8,
    marginBottom: 8,
  },
  reactLogo: {
    height: '100%',
    width: '100%',
    resizeMode: 'stretch',
  },
  buttonContainer: {
    marginTop: 40, // Adjust this value to place the button lower
    marginBottom: 20,
    marginHorizontal: 20,
  },
  button: {
    backgroundColor: '#4CAF50', // Green color for the button
    paddingVertical: 15,
    paddingHorizontal: 20,
    borderRadius: 25, // Rounded corners
    alignItems: 'center',
  },
  buttonText: {
    color: '#fff',
    fontSize: 18, // Bigger text
    fontWeight: 'bold',
  },
   messageContainer: {
    marginTop: 20,
    alignItems: 'center',
  },
});
